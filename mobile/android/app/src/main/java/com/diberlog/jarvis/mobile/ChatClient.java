package com.diberlog.jarvis.mobile;

import org.json.JSONArray;
import org.json.JSONObject;
import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;

/** Fixed HTTPS endpoints, no tools, no redirects, no automatic paid retry. */
final class ChatClient {
    static String ask(boolean openai, String key, String model, JSONArray messages) throws Exception {
        if (key.trim().isEmpty() || key.length() > 1024 || key.chars().anyMatch(Character::isISOControl))
            throw new IllegalArgumentException("Укажите корректный API-ключ в настройках.");
        if (model.trim().isEmpty() || model.length() > 128 || model.chars().anyMatch(Character::isISOControl))
            throw new IllegalArgumentException("Укажите название модели.");
        JSONObject payload = new JSONObject().put("model", model);
        if (openai) payload.put("input", messages).put("store", false).put("max_output_tokens", 2000);
        else payload.put("messages", messages).put("stream", false).put("max_tokens", 2000);
        HttpURLConnection connection = (HttpURLConnection) new URL(openai
            ? "https://api.openai.com/v1/responses" : "https://api.deepseek.com/chat/completions").openConnection();
        try {
            connection.setInstanceFollowRedirects(false);
            connection.setConnectTimeout(15000);
            connection.setReadTimeout(60000);
            connection.setRequestMethod("POST");
            connection.setDoOutput(true);
            connection.setRequestProperty("Authorization", "Bearer " + key);
            connection.setRequestProperty("Content-Type", "application/json; charset=utf-8");
            try (OutputStream output = connection.getOutputStream()) { output.write(payload.toString().getBytes(StandardCharsets.UTF_8)); }
            int status = connection.getResponseCode();
            if (status == 401) throw new IOException("API-ключ отклонён. Проверьте настройки.");
            if (status == 403) throw new IOException("Доступ к сервису или модели запрещён для аккаунта.");
            if (status == 429) throw new IOException("Лимит запросов или средств. Автоматического повтора нет.");
            if (status < 200 || status >= 300) throw new IOException("Сервис вернул HTTP " + status + ". Проверьте модель и доступность сервиса.");
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            try (InputStream input = connection.getInputStream()) {
                byte[] buffer = new byte[8192];
                int count;
                while ((count = input.read(buffer)) != -1) {
                    if (bytes.size() + count > 1024 * 1024) throw new IOException("Ответ слишком большой.");
                    bytes.write(buffer, 0, count);
                }
            }
            JSONObject result = new JSONObject(bytes.toString(StandardCharsets.UTF_8.name()));
            String text;
            if (openai) {
                if (!"completed".equals(result.optString("status"))) throw new IOException("Ответ не завершён. Сократите запрос; повтор может оплачиваться.");
                StringBuilder answer = new StringBuilder();
                JSONArray output = result.getJSONArray("output");
                for (int i = 0; i < output.length(); i++) {
                    JSONObject item = output.getJSONObject(i);
                    if (!"message".equals(item.optString("type"))) continue;
                    JSONArray content = item.optJSONArray("content");
                    if (content == null) continue;
                    for (int j = 0; j < content.length(); j++) {
                        JSONObject part = content.getJSONObject(j);
                        if ("output_text".equals(part.optString("type"))) answer.append(part.getString("text")).append('\n');
                        if ("refusal".equals(part.optString("type"))) answer.append(part.optString("refusal")).append('\n');
                    }
                }
                text = answer.toString().trim();
            } else {
                JSONObject choice = result.getJSONArray("choices").getJSONObject(0);
                if (!"stop".equals(choice.optString("finish_reason"))) throw new IOException("Ответ не завершён. Сократите запрос.");
                text = choice.getJSONObject("message").getString("content").trim();
            }
            if (text.isEmpty() || text.length() > 32000) throw new IOException("Сервис вернул пустой или слишком длинный ответ.");
            return text;
        } catch (SocketTimeoutException e) { throw new IOException("Время ожидания истекло. Повторный запрос может оплачиваться."); }
        catch (org.json.JSONException e) { throw new IOException("Некорректный формат ответа сервиса."); }
        finally { connection.disconnect(); }
    }
}
