package com.diberlog.jarvis.mobile;

import android.app.AlertDialog;
import android.widget.*;
import org.json.*;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicReference;

/** Explicit, expiring commands over the existing paired and certificate-pinned LAN channel. */
final class MobileCommands {
    static JSONObject call(MainActivity a,String path,JSONObject body)throws Exception {
        JSONObject pair=PrivateStorage.pairing(a);
        if(pair==null||!pair.has("token"))throw new Exception("Сначала подтвердите сопряжение с ПК");
        return SyncClient.request(a,pair,path,pair.getString("token"),body);
    }
    static String status(JSONObject response){
        switch(response.optString("status")){
            case "queued":return response.optBoolean("expired")?"Время команды истекло. Она не отправлена ядру.":"ПК получил команду. Ожидаем голосовое ядро…";
            case "claimed":case "uncertain":return "Команда передана ядру; результат пока неизвестен. Повторно она не запускается.";
            case "executed":return "Команда выполнена на ПК";
            case "shown":return "Раздел открыт в окне JARVIS на ПК";
            case "expired":return "Команда истекла или устройство отключено";
            default:return "Команда не выполнена. Проверьте приложение на ПК.";
        }
    }
    static void open(MainActivity a){AtomicReference<JSONArray> result=new AtomicReference<>();a.job(()->result.set(call(a,"/v2/commands",new JSONObject()).getJSONArray("commands")),()->{
        JSONArray commands=result.get();if(commands.length()==0){a.error("На этом ПК нет доступных команд");return;}
        LinearLayout list=a.column();list.setPadding(a.dp(16),a.dp(8),a.dp(16),a.dp(16));list.addView(a.label("Выполняются сразу на сопряжённом ПК. JARVIS и обмен должны быть включены. Команда действует одну минуту.",14));
        AlertDialog[] dialog={null};for(int n=0;n<commands.length();n++){JSONObject command=commands.optJSONObject(n);String id=command.optString("id"),title=command.optString("title",id);list.addView(a.button(title,()->{dialog[0].dismiss();send(a,id,title);}));}
        String last=a.prefs.getString("lastRemoteRequest","");if(!last.isEmpty())list.addView(a.button("Проверить последнюю команду",()->receipt(a,last,"Последняя команда")));
        ScrollView scroll=new ScrollView(a);scroll.addView(list);dialog[0]=new AlertDialog.Builder(a).setTitle("Команды компьютеру").setView(scroll).setNegativeButton("Закрыть",null).create();dialog[0].show();
    });}
    static void send(MainActivity a,String command,String title){String id=UUID.randomUUID().toString();a.prefs.edit().putString("lastRemoteRequest",id).apply();AtomicReference<JSONObject> response=new AtomicReference<>();a.job(()->response.set(call(a,"/v2/command",new JSONObject().put("id",id).put("command_id",command).put("expires_at",System.currentTimeMillis()/1000+60))),()->receipt(a,id,title));}
    static void receipt(MainActivity a,String id,String title){AtomicReference<JSONObject> result=new AtomicReference<>();a.job(()->result.set(call(a,"/v2/command/status",new JSONObject().put("id",id))),()->new AlertDialog.Builder(a).setTitle(title).setMessage(status(result.get())).setPositiveButton("Проверить результат",(d,w)->receipt(a,id,title)).setNegativeButton("Закрыть",null).show());}
}
