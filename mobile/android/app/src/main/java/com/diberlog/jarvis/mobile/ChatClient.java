package com.diberlog.jarvis.mobile;

import org.json.JSONArray;
import org.json.JSONObject;
import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;

/** Fixed HTTPS endpoints, no tools, no redirects, no automatic paid retry. */
final class ChatClient {
    static String defaultModel(boolean openai){return openai?"gpt-4.1-mini":"deepseek-flash";}
    static String ask(boolean openai, String key, String model, JSONArray messages) throws Exception { return ask(openai,key,model,messages,false); }
    static String ask(boolean openai, String key, String model, JSONArray messages,boolean full) throws Exception {
        if (key.trim().isEmpty() || key.length() > 1024 || key.chars().anyMatch(Character::isISOControl))
            throw new IllegalArgumentException("Укажите корректный API-ключ в настройках.");
        if (model.trim().isEmpty() || model.length() > 128 || model.chars().anyMatch(Character::isISOControl))
            throw new IllegalArgumentException("Укажите название модели.");
        JSONArray preparedMessages=new JSONArray().put(new JSONObject().put("role","system").put("content","Ты — JARVIS, помощник в мобильном приложении. Если нужен документ, подготовь его готовый текст: пользователь сможет открыть его как документ, отредактировать и сохранить в Word или заметки. Не утверждай, что файл уже сохранён или изменён на устройстве: это выполняется пользователем через интерфейс. У тебя нет доступа к сейфу, ключам, произвольным файлам и управлению устройствами."));for(int n=0;n<messages.length();n++)preparedMessages.put(messages.getJSONObject(n));
        JSONObject payload = new JSONObject().put("model", model);
        if (openai) payload.put("input", messages).put("store", false).put("max_output_tokens", 2000);
        else payload.put("messages", preparedMessages).put("stream", full).put("max_tokens", full?65536:8192).put("thinking",new JSONObject().put("type",full?"enabled":"disabled"));
        if(!openai&&full)payload.put("reasoning_effort","max");
        HttpURLConnection connection = (HttpURLConnection) new URL(openai
            ? "https://api.openai.com/v1/responses" : "https://api.deepseek.com/chat/completions").openConnection();
        try {
            connection.setInstanceFollowRedirects(false);
            connection.setConnectTimeout(15000);
            connection.setReadTimeout(full?180000:60000);
            connection.setRequestMethod("POST");
            connection.setDoOutput(true);
            connection.setRequestProperty("Authorization", "Bearer " + key);
            connection.setRequestProperty("Content-Type", "application/json; charset=utf-8");
            byte[] request=payload.toString().getBytes(StandardCharsets.UTF_8);connection.setFixedLengthStreamingMode(request.length);
            try (OutputStream output = connection.getOutputStream()) { output.write(request); }
            int status = connection.getResponseCode();
            if (status == 401) throw new IOException("API-ключ отклонён. Проверьте настройки.");
            if (status == 403) throw new IOException("Доступ к сервису или модели запрещён для аккаунта.");
            if (status == 429) throw new IOException("Лимит запросов или средств. Автоматического повтора нет.");
            if (status < 200 || status >= 300) throw new IOException("Сервис вернул HTTP " + status + ". Проверьте модель и доступность сервиса.");
            if(!openai&&full)return readStream(connection.getInputStream());
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            try (InputStream input = connection.getInputStream()) {
                byte[] buffer = new byte[8192];
                int count;
                while ((count = input.read(buffer)) != -1) {
                    if (bytes.size() + count > 8 * 1024 * 1024) throw new IOException("Ответ слишком большой.");
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
    static String readStream(InputStream input)throws Exception{
        StringBuilder text=new StringBuilder();boolean finished=false,done=false;long received=0;
        try(BufferedReader reader=new BufferedReader(new InputStreamReader(input,StandardCharsets.UTF_8))){String line;while((line=reader.readLine())!=null){received+=line.length();if(received>96*1024*1024||line.length()>2*1024*1024)throw new IOException("Ответ слишком большой");if(!line.startsWith("data:"))continue;String data=line.substring(5).trim();if(data.equals("[DONE]")){done=true;break;}if(data.isEmpty())continue;JSONObject packet=new JSONObject(data);if(packet.has("error"))throw new IOException("Сервис прервал ответ. Повторный запрос может оплачиваться.");JSONArray choices=packet.optJSONArray("choices");if(choices==null)continue;for(int n=0;n<choices.length();n++){JSONObject choice=choices.getJSONObject(n),delta=choice.optJSONObject("delta");if(delta!=null&&!delta.isNull("content"))text.append(delta.getString("content"));if(text.length()>1_000_000)throw new IOException("Ответ больше 1 000 000 символов");if(!choice.isNull("finish_reason")){if(!choice.getString("finish_reason").equals("stop"))throw new IOException("Ответ не завершён. Сократите запрос.");finished=true;}}}}
        if(!done||!finished||text.toString().trim().isEmpty())throw new IOException("Ответ прерван или не завершён. Автоматического повтора нет.");return text.toString().trim();
    }
}
