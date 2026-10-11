package com.diberlog.jarvis.mobile;

import android.app.*;
import android.content.*;
import android.os.Bundle;
import android.graphics.Color;
import android.speech.RecognizerIntent;
import android.speech.tts.TextToSpeech;
import android.text.InputType;
import android.view.*;
import android.widget.*;
import org.json.*;
import java.util.*;
import java.util.concurrent.*;

/** Intentionally separate from the desktop engine: no shell, COM or background microphone. */
public final class MainActivity extends Activity {
    private LinearLayout page;
    private EditText input, note;
    private TextView conversation;
    private Button send;
    private String provider = "OpenAI", model = "gpt-4.1-mini", key = "", lastAnswer = "", draft = "";
    private JSONArray messages = new JSONArray();
    private JSONArray tasks = new JSONArray();
    private SharedPreferences preferences;
    private TextToSpeech speaker;
    private boolean speechReady, busy;
    private int tab;
    private final ExecutorService worker = Executors.newSingleThreadExecutor();
    private static final int VOICE = 1;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        preferences = getSharedPreferences("jarvis-mobile", MODE_PRIVATE);
        provider = preferences.getString("provider", "OpenAI");
        model = preferences.getString("model", "gpt-4.1-mini");
        try { messages = new JSONArray(preferences.getString("messages", "[]")); tasks = new JSONArray(preferences.getString("tasks", "[]")); }
        catch (JSONException e) { toast("История повреждена. Сохранённые данные не будут заменены до явного изменения."); }
        if (state != null) { draft = state.getString("draft", ""); tab = state.getInt("tab", 0); }
        speaker = new TextToSpeech(this, status -> {
            if (status == TextToSpeech.SUCCESS) {
                int language = speaker.setLanguage(new Locale("ru", "RU"));
                speechReady = language != TextToSpeech.LANG_MISSING_DATA && language != TextToSpeech.LANG_NOT_SUPPORTED;
            }
        });
        show(tab);
    }

    private void show(int selected) {
        if (input != null) draft = input.getText().toString();
        input = null; note = null; conversation = null; send = null; tab = selected;
        LinearLayout root = new LinearLayout(this); root.setOrientation(LinearLayout.VERTICAL);
        root.setPadding(24, 24, 24, 24); root.setBackgroundColor(Color.rgb(8, 26, 31));
        root.setOnApplyWindowInsetsListener((view, insets) -> {
            view.setPadding(24 + insets.getSystemWindowInsetLeft(), 24 + insets.getSystemWindowInsetTop(),
                24 + insets.getSystemWindowInsetRight(), 24 + insets.getSystemWindowInsetBottom());
            return insets;
        });
        TextView header = label("JARVIS · MOBILE LAB", 24); root.addView(header);
        LinearLayout tabs = new LinearLayout(this);
        String[] names = {"Чат", "Заметка", "Задачи", "Настройки"};
        for (int i = 0; i < names.length; i++) { final int next = i; Button b = button(names[i], () -> show(next)); tabs.addView(b, new LinearLayout.LayoutParams(0, -2, 1)); }
        root.addView(tabs);
        ScrollView scroll = new ScrollView(this); page = new LinearLayout(this); page.setOrientation(LinearLayout.VERTICAL); scroll.addView(page);
        root.addView(scroll, new LinearLayout.LayoutParams(-1, 0, 1)); setContentView(root);
        if (selected == 0) chat(); else if (selected == 1) note(); else if (selected == 2) tasks(); else settings();
    }

    private TextView label(String text, int size) {
        TextView view = new TextView(this); view.setText(text); view.setTextSize(size); view.setTextColor(Color.rgb(220, 249, 250));
        view.setPadding(0, 12, 0, 12); view.setTextIsSelectable(true); return view;
    }
    private EditText field(String hint, String value, boolean secret) {
        EditText view = new EditText(this); view.setHint(hint); view.setText(value); view.setTextColor(Color.WHITE); view.setHintTextColor(Color.LTGRAY);
        if (secret) { view.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_VARIATION_PASSWORD); view.setSaveEnabled(false); }
        else view.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE);
        page.addView(view); return view;
    }
    private Button button(String text, Runnable action) { Button view = new Button(this); view.setText(text); view.setOnClickListener(v -> action.run()); return view; }
    private void action(String text, Runnable action) { page.addView(button(text, action)); }
    private void toast(String text) { Toast.makeText(this, text, Toast.LENGTH_LONG).show(); }

    private void chat() {
        page.addView(label("Запрос и контекст передаются " + provider + ". Возможны расходы. Модель не выполняет действия на телефоне.", 14));
        conversation = label(transcript(), 16); page.addView(conversation);
        input = field("Напишите или продиктуйте запрос", draft, false);
        action("Продиктовать", this::dictate);
        send = button(busy ? "Ожидание ответа…" : "Отправить запрос", this::ask); send.setEnabled(!busy); page.addView(send);
        action("Озвучить последний ответ", () -> { if (speechReady && !lastAnswer.isEmpty()) speaker.speak(lastAnswer, TextToSpeech.QUEUE_FLUSH, null, "jarvis-answer"); else toast("Ответ или русский системный голос недоступен."); });
        action("Поделиться историей", () -> share(transcript()));
        action("Очистить историю", () -> new AlertDialog.Builder(this).setMessage("Удалить историю чата на телефоне?")
            .setPositiveButton("Удалить", (dialog, which) -> { if (busy) { toast("Дождитесь ответа."); return; } messages = new JSONArray(); lastAnswer = ""; saveMessages(); show(0); })
            .setNegativeButton("Отмена", null).show());
    }

    private void ask() {
        if (busy) return;
        String text = input.getText().toString().trim();
        if (text.isEmpty() || text.length() > 4000) { toast("Введите запрос до 4000 символов."); return; }
        if (key.isEmpty()) { toast("В настройках укажите свой API-ключ. Он хранится только в памяти этого сеанса."); return; }
        try {
            messages.put(new JSONObject().put("role", "user").put("content", text)); trimHistory(); saveMessages();
            JSONArray context = new JSONArray();
            for (int i = Math.max(0, messages.length() - 20); i < messages.length(); i++) context.put(messages.getJSONObject(i));
            String requestKey = key, requestModel = model; boolean openai = "OpenAI".equals(provider);
            busy = true; draft = ""; input.setText(""); conversation.setText(transcript()); send.setEnabled(false); send.setText("Ожидание ответа…");
            worker.execute(() -> {
                try { String answer = ChatClient.ask(openai, requestKey, requestModel, context);
                    runOnUiThread(() -> { if (isFinishing() || isDestroyed()) return; try { messages.put(new JSONObject().put("role", "assistant").put("content", answer)); lastAnswer = answer; trimHistory(); saveMessages(); } catch (JSONException ignored) {} finishRequest(null); });
                } catch (Exception error) { runOnUiThread(() -> { if (!isFinishing() && !isDestroyed()) finishRequest(error instanceof java.io.IOException || error instanceof IllegalArgumentException ? error.getMessage() : "Не удалось выполнить запрос. Проверьте интернет и настройки."); }); }
            });
        } catch (JSONException e) { toast("Не удалось подготовить запрос."); }
    }
    private void finishRequest(String error) { busy = false; if (tab == 0) { if (conversation != null) conversation.setText(transcript()); if (send != null) { send.setEnabled(true); send.setText("Отправить запрос"); } } if (error != null) new AlertDialog.Builder(this).setTitle("Запрос не выполнен").setMessage(error).setPositiveButton("OK", null).show(); }
    private void trimHistory() throws JSONException { if (messages.length() > 100) { JSONArray recent = new JSONArray(); for (int i = messages.length() - 100; i < messages.length(); i++) recent.put(messages.getJSONObject(i)); messages = recent; } }
    private void saveMessages() { preferences.edit().putString("messages", messages.toString()).apply(); }
    private String transcript() { StringBuilder text = new StringBuilder(); for (int i = 0; i < messages.length(); i++) { JSONObject message = messages.optJSONObject(i); if (message != null) text.append("user".equals(message.optString("role")) ? "Вы: " : "JARVIS: ").append(message.optString("content")).append("\n\n"); } return text.toString(); }

    private void note() {
        page.addView(label("Локальная заметка. Не шифруется и не отправляется модели автоматически.", 14));
        note = field("Текст заметки", preferences.getString("note", ""), false); note.setMinLines(6);
        action("Сохранить", () -> { String text = note.getText().toString(); if (text.length() > 12000) { toast("Лимит заметки — 12 000 символов."); return; } preferences.edit().putString("note", text).apply(); toast("Заметка сохранена."); });
        action("Продиктовать в заметку", this::dictate);
        action("Поделиться заметкой", () -> share(note.getText().toString()));
    }
    private void tasks() {
        EditText task = field("Новая задача", "", false);
        action("Добавить задачу", () -> {
            String text = task.getText().toString().trim();
            if (text.isEmpty() || text.length() > 500 || tasks.length() >= 100) { toast("До 100 задач, до 500 символов в каждой."); return; }
            try { tasks.put(new JSONObject().put("text", text).put("done", false)); saveTasks(); show(2); } catch (JSONException ignored) {}
        });
        for (int i = 0; i < tasks.length(); i++) {
            JSONObject item = tasks.optJSONObject(i); if (item == null) continue; final int index = i;
            CheckBox check = new CheckBox(this); check.setText(item.optString("text")); check.setTextColor(Color.WHITE); check.setChecked(item.optBoolean("done"));
            check.setOnCheckedChangeListener((v, checked) -> { try { tasks.getJSONObject(index).put("done", checked); saveTasks(); } catch (JSONException ignored) {} }); page.addView(check);
        }
        action("Убрать выполненные", () -> { JSONArray remaining = new JSONArray(); for (int i = 0; i < tasks.length(); i++) { JSONObject item = tasks.optJSONObject(i); if (item != null && !item.optBoolean("done")) remaining.put(item); } tasks = remaining; saveTasks(); show(2); });
        action("Поделиться задачами", () -> share(tasks.toString()));
        page.addView(label("Это список задач, не фоновый планировщик. Уведомления и напоминания ещё не перенесены.", 14));
    }
    private void saveTasks() { preferences.edit().putString("tasks", tasks.toString()).apply(); }
    private void settings() {
        page.addView(label("Мобильный прототип: нет Outlook COM, управления программами, сейфа, вложений и фонового прослушивания. Локальная Ollama пока не подключена.", 14));
        Spinner providers = new Spinner(this); providers.setAdapter(new ArrayAdapter<>(this, android.R.layout.simple_spinner_dropdown_item, new String[]{"OpenAI", "DeepSeek"})); providers.setSelection("OpenAI".equals(provider) ? 0 : 1); page.addView(providers);
        EditText modelField = field("Модель: gpt-4.1-mini / deepseek-chat", model, false);
        EditText keyField = field("Новый API-ключ (только на текущий сеанс)", "", true);
        action("Сохранить настройки", () -> {
            String selected = providers.getSelectedItem().toString(); String nextModel = modelField.getText().toString().trim();
            String nextKey = keyField.getText().toString().trim();
            if (nextModel.isEmpty() || nextModel.length() > 128 || nextKey.length() > 1024) { toast("Проверьте модель и ключ."); return; }
            if (!selected.equals(provider)) key = "";
            provider = selected; model = nextModel; if (!nextKey.isEmpty()) key = nextKey;
            preferences.edit().putString("provider", provider).putString("model", model).apply(); keyField.setText(""); toast("Сохранено. Ключ не записан на диск.");
        });
        action("Забыть API-ключ", () -> { key = ""; keyField.setText(""); toast("Ключ удалён из текущего сеанса."); });
        page.addView(label("История, заметка и задачи сохраняются на телефоне без шифрования. Системное распознавание может использовать сеть; офлайн зависит от установленного сервиса. После пересоздания приложения API-ключ нужно ввести заново.", 14));
    }
    private void dictate() {
        Intent intent = new Intent(RecognizerIntent.ACTION_RECOGNIZE_SPEECH).putExtra(RecognizerIntent.EXTRA_LANGUAGE_MODEL, RecognizerIntent.LANGUAGE_MODEL_FREE_FORM)
            .putExtra(RecognizerIntent.EXTRA_LANGUAGE, "ru-RU").putExtra(RecognizerIntent.EXTRA_PROMPT, "Продиктуйте текст; ничего не будет отправлено автоматически");
        try { startActivityForResult(intent, VOICE); } catch (ActivityNotFoundException e) { toast("На телефоне нет сервиса распознавания речи."); }
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == VOICE && result == RESULT_OK && data != null) {
            ArrayList<String> resultText = data.getStringArrayListExtra(RecognizerIntent.EXTRA_RESULTS);
            if (resultText == null || resultText.isEmpty()) return;
            EditText target = tab == 1 ? note : input;
            if (target != null) target.append((target.length() == 0 ? "" : "\n") + resultText.get(0));
        }
    }
    private void share(String text) { if (text.isEmpty()) { toast("Нет текста для экспорта."); return; } startActivity(Intent.createChooser(new Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text), "Экспорт текста")); }
    @Override protected void onSaveInstanceState(Bundle state) { if (input != null) draft = input.getText().toString(); state.putString("draft", draft); state.putInt("tab", tab); super.onSaveInstanceState(state); }
    @Override protected void onDestroy() { worker.shutdownNow(); key = ""; if (speaker != null) { speaker.stop(); speaker.shutdown(); } super.onDestroy(); }
}
