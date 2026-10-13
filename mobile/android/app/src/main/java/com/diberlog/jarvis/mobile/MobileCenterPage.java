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
            for(int n=0;n<notes.length();n++){JSONObject existing=notes.getJSONObject(n);if(!existing.getString("id").equals(id)){next.put(existing);continue;}found=true;if(note!=null){JSONObject merged=new JSONObject(existing.toString());for(Iterator<String> keys=note.keys();keys.hasNext();){String key=keys.next();merged.put(key,note.get(key));}if(note.optString("type").equals("checklist"))merged.remove("html");else merged.remove("items");next.put(merged);}}
            if(note!=null&&!found)next.put(note);base.put("notes",next);
        }
        MobileStore.save(activity,base);base=MobileStore.center(activity);return new JSONObject().put("ok",true).put("center",publicCenter()).toString();
    }catch(Exception e){try{if(checkpoint==null)throw new IOException("Нет исходных данных");JSONObject previous=new JSONObject(checkpoint);ArrayList<String> remove=new ArrayList<>();for(Iterator<String> keys=base.keys();keys.hasNext();)remove.add(keys.next());for(String name:remove)base.remove(name);for(Iterator<String> names=previous.keys();names.hasNext();){String name=names.next();base.put(name,previous.get(name));}}catch(Exception ignored){}return failure(e);}}
    @JavascriptInterface public synchronized String timer(String raw){try{
        if(!active)throw new IOException("Раздел закрыт");JSONObject state;if(raw.isEmpty()){state=SyncJournal.snapshot(activity).optJSONObject("training_timer");if(state==null)state=new JSONObject().put("label","Отдых").put("total",120).put("remaining",120).put("deadline",JSONObject.NULL).put("phase","idle");}
        else{if(raw.length()>2000)throw new IOException("Некорректный таймер");state=new JSONObject(raw);int total=state.getInt("total"),remaining=state.getInt("remaining");String phase=state.getString("phase");if(total<0||total>86400||remaining<0||remaining>86400||!Arrays.asList("idle","running","paused","finished").contains(phase)||state.getString("label").length()>200)throw new IOException("Некорректный таймер");SyncJournal.save(activity,"training_timer",state);if(phase.equals("running")&&!state.isNull("deadline"))ReminderReceiver.alarm(activity,"training-rest",state.getString("label"),state.getLong("deadline"));else ReminderReceiver.cancel(activity,"training-rest");}
        return state.toString();
    }catch(Exception e){return failure(e);}}
    void close(){active=false;view.removeJavascriptInterface("NativeCenter");view.stopLoading();view.destroy();}
    @JavascriptInterface public void pickImage(){if(active&&mode.equals("notes"))activity.runOnUiThread(()->activity.pickNoteImage(this));}
    void photoResult(android.net.Uri uri){
        if(!active)return;if(uri==null){photoCallback(new JSONObject());return;}
        new Thread(()->{android.graphics.Bitmap bitmap=null,scaled=null,flat=null;try{
            String type=activity.getContentResolver().getType(uri);if(type==null||!Arrays.asList("image/png","image/jpeg","image/webp","image/gif","image/bmp").contains(type))throw new IOException("Выберите PNG, JPEG, WebP, GIF или BMP");
            byte[] bytes;try(InputStream input=activity.getContentResolver().openInputStream(uri)){if(input==null)throw new IOException("Картинка не открылась");bytes=ChatDocuments.bounded(input,10_000_000);}
            android.graphics.BitmapFactory.Options options=new android.graphics.BitmapFactory.Options();options.inJustDecodeBounds=true;android.graphics.BitmapFactory.decodeByteArray(bytes,0,bytes.length,options);
            if(options.outWidth<1||options.outHeight<1||options.outWidth>40000||options.outHeight>40000)throw new IOException("Не удалось прочитать размер картинки");int sample=1;while(options.outWidth/sample>1600||options.outHeight/sample>1600)sample*=2;options.inSampleSize=sample;options.inJustDecodeBounds=false;
            bitmap=android.graphics.BitmapFactory.decodeByteArray(bytes,0,bytes.length,options);if(bitmap==null)throw new IOException("Не удалось прочитать картинку");scaled=bitmap;flat=android.graphics.Bitmap.createBitmap(scaled.getWidth(),scaled.getHeight(),android.graphics.Bitmap.Config.ARGB_8888);android.graphics.Canvas canvas=new android.graphics.Canvas(flat);canvas.drawColor(android.graphics.Color.WHITE);canvas.drawBitmap(scaled,0,0,new android.graphics.Paint(3));ByteArrayOutputStream out=new ByteArrayOutputStream();flat.compress(android.graphics.Bitmap.CompressFormat.JPEG,85,out);if(out.size()>2_000_000)throw new IOException("Картинка слишком большая: выберите меньший файл");
            photoCallback(new JSONObject().put("src","data:image/jpeg;base64,"+android.util.Base64.encodeToString(out.toByteArray(),android.util.Base64.NO_WRAP)));
        }catch(Exception e){try{photoCallback(new JSONObject().put("error",e.getMessage()==null?"Не удалось добавить картинку":e.getMessage()));}catch(JSONException ignored){}}
        finally{if(flat!=null)flat.recycle();if(bitmap!=null)bitmap.recycle();}},"JarvisPhoto").start();
    }
    void photoCallback(JSONObject result){activity.runOnUiThread(()->{if(active)view.evaluateJavascript("window.dispatchEvent(new CustomEvent('jarvis-image',{detail:"+result.toString()+"}))",null);});}
}
