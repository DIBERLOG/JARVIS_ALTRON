package com.diberlog.jarvis.mobile;

import android.webkit.*;
import android.view.*;
import android.widget.*;
import android.app.AlertDialog;
import org.json.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.time.LocalDate;
import java.util.*;

/** Bundled desktop editors. No network, file access, browser storage, keys or vault bridge. */
final class MobileCenterPage {
    final MainActivity activity;
    final WebView view;
    final String mode;
    JSONObject base;
    volatile boolean active=true,dirty;
    byte[] exportBytes;
    MobileCenterPage(MainActivity activity,String mode)throws Exception{
        this.activity=activity;this.mode=mode;view=new WebView(activity);view.setBackgroundColor(MainActivity.BG);view.setVerticalScrollBarEnabled(false);
        WebSettings settings=view.getSettings();settings.setJavaScriptEnabled(true);settings.setAllowFileAccess(false);settings.setAllowContentAccess(false);settings.setBlockNetworkLoads(true);settings.setDomStorageEnabled(false);settings.setDatabaseEnabled(false);settings.setSaveFormData(false);settings.setCacheMode(WebSettings.LOAD_NO_CACHE);settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);settings.setSafeBrowsingEnabled(true);settings.setTextZoom(Math.round(activity.getResources().getConfiguration().fontScale*100));
        view.setWebViewClient(new WebViewClient(){@Override public boolean shouldOverrideUrlLoading(WebView v,WebResourceRequest r){return true;}@Override public WebResourceResponse shouldInterceptRequest(WebView v,WebResourceRequest r){String url=r.getUrl().toString();if(url.startsWith("data:image/png;base64,")||url.startsWith("data:image/jpeg;base64,")||url.startsWith("data:image/webp;base64,")||url.startsWith("data:image/gif;base64,"))return null;return new WebResourceResponse("text/plain","UTF-8",new ByteArrayInputStream(new byte[0]));}});
        view.setWebChromeClient(new WebChromeClient(){
            @Override public void onPermissionRequest(PermissionRequest request){request.deny();}
            @Override public void onGeolocationPermissionsShowPrompt(String origin,GeolocationPermissions.Callback callback){callback.invoke(origin,false,false);}
            @Override public boolean onJsConfirm(WebView v,String url,String message,JsResult result){new AlertDialog.Builder(activity).setMessage(message).setPositiveButton("Продолжить",(d,n)->result.confirm()).setNegativeButton("Остаться",(d,n)->result.cancel()).setOnCancelListener(d->result.cancel()).show();return true;}
            @Override public boolean onJsAlert(WebView v,String url,String message,JsResult result){new AlertDialog.Builder(activity).setMessage(message).setPositiveButton("Понятно",(d,n)->result.confirm()).setOnCancelListener(d->result.cancel()).show();return true;}
            @Override public boolean onJsPrompt(WebView v,String url,String message,String value,JsPromptResult result){EditText input=new EditText(activity);input.setText(value);new AlertDialog.Builder(activity).setMessage(message).setView(input).setPositiveButton("Готово",(d,n)->result.confirm(input.getText().toString())).setNegativeButton("Отмена",(d,n)->result.cancel()).setOnCancelListener(d->result.cancel()).show();return true;}
            @Override public boolean onShowFileChooser(WebView v,ValueCallback<android.net.Uri[]> callback,FileChooserParams params){activity.chooseNoteImages(callback);return true;}
        });
        view.addJavascriptInterface(this,"NativeCenter");
        String nonce=UUID.randomUUID().toString();String css=asset("mobile-center.css"),js=asset("mobile-center.js").replace("</script","<\\/script");
        String html="<!doctype html><html lang='ru' data-theme='"+(MobileTheme.light(activity)?"light":"dark")+"'><head><meta name='viewport' content='width=device-width,initial-scale=1'><meta http-equiv='Content-Security-Policy' content=\"default-src 'none'; script-src 'nonce-"+nonce+"'; style-src 'unsafe-inline'; img-src data: blob:; connect-src 'none'; frame-src 'none'; object-src 'none'; base-uri 'none'; form-action 'none'\"><style>"+css+"</style></head><body><div id='app'></div><script nonce='"+nonce+"'>const JarvisMode="+JSONObject.quote(mode)+";"+js+"</script></body></html>";
        view.loadDataWithBaseURL("https://jarvis.invalid/mobile/",html,"text/html","UTF-8",null);
    }
    String asset(String name)throws Exception{try(InputStream in=activity.getAssets().open(name)){return PublicData.read(in,8*1024*1024);}}
    String failure(Exception e){try{return new JSONObject().put("ok",false).put("error",e.getMessage()==null?"Не удалось сохранить; исходные данные сохранены":e.getMessage()).toString();}catch(JSONException impossible){return "{\"ok\":false,\"error\":\"Не удалось сохранить\"}";}}
    JSONObject publicCenter()throws JSONException{return new JSONObject().put("notes",base.getJSONArray("notes")).put("training",base.opt("training")).put("habits",base.getJSONArray("habits"));}
    @JavascriptInterface public void dirty(boolean value){if(active)dirty=value;}
    @JavascriptInterface public synchronized String load(){try{if(!active)throw new IOException("Раздел закрыт");base=MobileStore.center(activity);return publicCenter().toString();}catch(Exception e){return failure(e);}}
    @JavascriptInterface public synchronized String save(String raw){String checkpoint=base==null?null:base.toString();try{
        if(!active||base==null)throw new IOException("Обновите раздел перед сохранением");if(raw.getBytes(StandardCharsets.UTF_8).length>16*1024*1024)throw new IOException("Запись превышает лимит 16 МБ");JSONObject request=new JSONObject(raw);
        if(mode.equals("habits")){if(request.length()!=1||!request.has("habits"))throw new IOException("Некорректный запрос привычек");base.put("habits",request.getJSONArray("habits"));}else if(mode.equals("training")){
            if(!request.has("training")||request.length()>2||request.has("habit")&&!Arrays.asList("Сходил в зал","Сделал зарядку").contains(request.optString("habit")))throw new IOException("Некорректный запрос тренировки");
            base.put("training",request.getJSONObject("training"));String habit=request.optString("habit");if(!habit.isEmpty()){JSONArray habits=base.getJSONArray("habits");String today=LocalDate.now().toString();for(int n=0;n<habits.length();n++){JSONObject item=habits.getJSONObject(n);if(item.optString("title").equals(habit)&&!item.has("archivedOn")&&item.optString("createdOn").compareTo(today)<=0){JSONObject entries=item.optJSONObject("entries");if(entries==null)entries=new JSONObject();entries.put(today,item.optInt("target",1));item.put("entries",entries);}}}
        }else{
            if(request.length()!=1||(!request.has("note")&&!request.has("deleteNote")))throw new IOException("Некорректный запрос заметки");JSONArray notes=base.getJSONArray("notes"),next=new JSONArray();JSONObject note=request.optJSONObject("note");String id=note==null?request.getString("deleteNote"):note.getString("id");if(id.isEmpty()||id.length()>128)throw new IOException("Некорректный ID заметки");if(note!=null&&(note.optString("title").trim().isEmpty()||note.optString("title").length()>500))throw new IOException("Укажите заголовок до 500 символов");boolean found=false;
            for(int n=0;n<notes.length();n++){JSONObject existing=notes.getJSONObject(n);if(!existing.getString("id").equals(id)){next.put(existing);continue;}found=true;if(note!=null){JSONObject merged=new JSONObject(existing.toString());for(Iterator<String> keys=note.keys();keys.hasNext();){String key=keys.next();merged.put(key,note.get(key));}if(note.optString("type").equals("mindmap")){merged.remove("html");merged.remove("items");}else{merged.remove("mindMap");if(note.optString("type").equals("checklist"))merged.remove("html");else merged.remove("items");}next.put(merged);}}
            if(note!=null&&!found)next.put(note);base.put("notes",next);
        }
        MobileStore.save(activity,base);base=MobileStore.center(activity);return new JSONObject().put("ok",true).put("center",publicCenter()).toString();
    }catch(Exception e){try{if(checkpoint==null)throw new IOException("Нет исходных данных");JSONObject previous=new JSONObject(checkpoint);ArrayList<String> remove=new ArrayList<>();for(Iterator<String> keys=base.keys();keys.hasNext();)remove.add(keys.next());for(String name:remove)base.remove(name);for(Iterator<String> names=previous.keys();names.hasNext();){String name=names.next();base.put(name,previous.get(name));}}catch(Exception ignored){}return failure(e);}}
    @JavascriptInterface public synchronized String timer(String raw){try{
        if(!active)throw new IOException("Раздел закрыт");JSONObject state;if(raw.isEmpty()){state=SyncJournal.snapshot(activity).optJSONObject("training_timer");if(state==null)state=new JSONObject().put("label","Отдых").put("total",120).put("remaining",120).put("deadline",JSONObject.NULL).put("phase","idle");}
        else{if(raw.length()>2000)throw new IOException("Некорректный таймер");state=new JSONObject(raw);int total=state.getInt("total"),remaining=state.getInt("remaining");String phase=state.getString("phase");if(total<0||total>86400||remaining<0||remaining>86400||!Arrays.asList("idle","running","paused","finished").contains(phase)||state.getString("label").length()>200)throw new IOException("Некорректный таймер");SyncJournal.save(activity,"training_timer",state);if(phase.equals("running")&&!state.isNull("deadline"))ReminderReceiver.alarm(activity,"training-rest",state.getString("label"),state.getLong("deadline"));else ReminderReceiver.cancel(activity,"training-rest");}
        return state.toString();
    }catch(Exception e){return failure(e);}}
    void close(){active=false;exportBytes=null;view.removeJavascriptInterface("NativeCenter");view.stopLoading();view.destroy();}
    @JavascriptInterface public void pickImage(){if(active&&mode.equals("notes"))activity.runOnUiThread(()->activity.pickNoteImage(this));}
    @JavascriptInterface public void pickAttachment(){if(active&&mode.equals("notes"))activity.runOnUiThread(()->{activity.attachmentTarget=this;try{activity.startActivityForResult(new android.content.Intent(android.content.Intent.ACTION_OPEN_DOCUMENT).setType("*/*").addCategory(android.content.Intent.CATEGORY_OPENABLE).putExtra(android.content.Intent.EXTRA_MIME_TYPES,new String[]{"application/pdf","text/plain","text/markdown","application/vnd.openxmlformats-officedocument.wordprocessingml.document"}),12);}catch(Exception e){activity.attachmentTarget=null;attachmentCallback(errorResult("Выбор файла недоступен"));}});}
    JSONObject errorResult(String error){JSONObject result=new JSONObject();try{result.put("error",error);}catch(JSONException ignored){}return result;}
    static String attachmentMime(String name)throws IOException{String lower=name.toLowerCase(Locale.ROOT);if(lower.endsWith(".pdf"))return "application/pdf";if(lower.endsWith(".txt"))return "text/plain";if(lower.endsWith(".md"))return "text/markdown";if(lower.endsWith(".docx"))return "application/vnd.openxmlformats-officedocument.wordprocessingml.document";throw new IOException("Выберите PDF, TXT, Markdown или Word (.docx)");}
    void attachmentResult(android.net.Uri uri){if(!active)return;if(uri==null){attachmentCallback(new JSONObject());return;}new Thread(()->{try{String name=ChatDocuments.displayName(activity,uri),mime=attachmentMime(name);byte[] bytes;try(InputStream in=activity.getContentResolver().openInputStream(uri)){if(in==null)throw new IOException("Файл не открылся");bytes=ChatDocuments.bounded(in,700_000);}if(bytes.length==0)throw new IOException("Файл пуст");attachmentCallback(new JSONObject().put("file",new JSONObject().put("name",name).put("mime",mime).put("size",bytes.length).put("data",android.util.Base64.encodeToString(bytes,android.util.Base64.NO_WRAP))));}catch(Exception e){attachmentCallback(errorResult(e.getMessage()==null?"Не удалось добавить файл":e.getMessage()));}},"JarvisMapFile").start();}
    void attachmentCallback(JSONObject result){activity.runOnUiThread(()->{if(active)view.evaluateJavascript("window.dispatchEvent(new CustomEvent('jarvis-attachment',{detail:"+result.toString()+"}))",null);});}
    @JavascriptInterface public void exportAttachment(String raw){if(!active||!mode.equals("notes"))return;try{if(raw.length()>950_000)throw new IOException("Файл слишком большой");JSONObject file=new JSONObject(raw);String name=file.getString("name"),mime=attachmentMime(name);if(name.length()>160||!mime.equals(file.getString("mime")))throw new IOException("Неверный формат файла");byte[] bytes=android.util.Base64.decode(file.getString("data"),android.util.Base64.NO_WRAP);if(bytes.length<1||bytes.length>700_000||bytes.length!=file.getInt("size"))throw new IOException("Файл повреждён");String safe=name.replaceAll("[\\\\/:*?\"<>|]","_");activity.runOnUiThread(()->{if(!active)return;exportBytes=bytes;activity.attachmentExportTarget=this;try{activity.startActivityForResult(new android.content.Intent(android.content.Intent.ACTION_CREATE_DOCUMENT).setType(mime).addCategory(android.content.Intent.CATEGORY_OPENABLE).putExtra(android.content.Intent.EXTRA_TITLE,safe),13);}catch(Exception e){exportBytes=null;activity.attachmentExportTarget=null;activity.error("Сохранение файла недоступно");}});}catch(Exception e){activity.runOnUiThread(()->activity.error(e.getMessage()));}}
    void exportResult(android.net.Uri uri){byte[] bytes=exportBytes;exportBytes=null;if(uri==null||bytes==null)return;activity.job(()->{try(OutputStream out=activity.getContentResolver().openOutputStream(uri)){if(out==null)throw new IOException("Файл не открылся");out.write(bytes);}},()->Toast.makeText(activity,"Файл сохранён",Toast.LENGTH_SHORT).show());}
    void photoResult(android.net.Uri uri){
        if(!active)return;if(uri==null){photoCallback(new JSONObject());return;}
        new Thread(()->{android.graphics.Bitmap bitmap=null,scaled=null,flat=null;try{
            String type=activity.getContentResolver().getType(uri);if(type==null||!type.startsWith("image/"))throw new IOException("Выберите изображение");
            android.graphics.BitmapFactory.Options options=new android.graphics.BitmapFactory.Options();options.inJustDecodeBounds=true;
            try(InputStream input=activity.getContentResolver().openInputStream(uri)){if(input==null)throw new IOException("Картинка не открылась");android.graphics.BitmapFactory.decodeStream(input,null,options);}
            if(options.outWidth<1||options.outHeight<1)throw new IOException("Этот формат изображения не поддерживается устройством");int sample=1;while(options.outWidth/sample>1600||options.outHeight/sample>1600)sample*=2;options.inSampleSize=sample;options.inJustDecodeBounds=false;
            try(InputStream input=activity.getContentResolver().openInputStream(uri)){bitmap=android.graphics.BitmapFactory.decodeStream(input,null,options);}if(bitmap==null)throw new IOException("Не удалось прочитать картинку");scaled=bitmap;flat=android.graphics.Bitmap.createBitmap(scaled.getWidth(),scaled.getHeight(),android.graphics.Bitmap.Config.ARGB_8888);android.graphics.Canvas canvas=new android.graphics.Canvas(flat);canvas.drawColor(android.graphics.Color.WHITE);canvas.drawBitmap(scaled,0,0,new android.graphics.Paint(3));ByteArrayOutputStream out=new ByteArrayOutputStream();flat.compress(android.graphics.Bitmap.CompressFormat.JPEG,85,out);
            for(int attempt=0;out.size()>440_000&&attempt<8;attempt++){android.graphics.Bitmap smaller=android.graphics.Bitmap.createScaledBitmap(flat,Math.max(1,flat.getWidth()*4/5),Math.max(1,flat.getHeight()*4/5),true);if(smaller!=flat)flat.recycle();flat=smaller;out.reset();flat.compress(android.graphics.Bitmap.CompressFormat.JPEG,80,out);}if(out.size()>440_000)throw new IOException("Не удалось уменьшить изображение");
            photoCallback(new JSONObject().put("src","data:image/jpeg;base64,"+android.util.Base64.encodeToString(out.toByteArray(),android.util.Base64.NO_WRAP)));
        }catch(Exception e){try{photoCallback(new JSONObject().put("error",e.getMessage()==null?"Не удалось добавить картинку":e.getMessage()));}catch(JSONException ignored){}}
        finally{if(flat!=null)flat.recycle();if(bitmap!=null)bitmap.recycle();}},"JarvisPhoto").start();
    }
    void photoCallback(JSONObject result){activity.runOnUiThread(()->{if(active)view.evaluateJavascript("window.dispatchEvent(new CustomEvent('jarvis-image',{detail:"+result.toString()+"}))",null);});}
}
