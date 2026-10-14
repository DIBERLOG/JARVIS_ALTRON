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
    static void page(MainActivity a)throws Exception{
        JSONObject pair=PrivateStorage.pairing(a);
        if(pair==null||!pair.has("token")){a.remoteCommands=null;a.text("Подключите телефон к компьютеру и подтвердите сопряжение на ПК. После этого здесь появятся доступные команды.");a.action("Подключить компьютер",()->a.show(12));return;}
        a.text("Компьютер: "+pair.optString("host")+":"+pair.optInt("port"));
        a.text("Выберите команду — она сразу отправится на сопряжённый ПК. JARVIS и обмен должны быть включены. Срок команды — одна минута.");
        a.action(a.remoteCommands==null?"Загрузить команды компьютера":"Обновить список команд",()->refresh(a));
        JSONArray commands=a.remoteCommands;
        if(commands!=null){if(commands.length()==0)a.text("На этом ПК пока нет доступных команд.");else for(int n=0;n<commands.length();n++){JSONObject command=commands.getJSONObject(n);String id=command.getString("id"),title=command.optString("title",id);Button button=a.button(title,()->send(a,id,title));button.setTag("remote-command-"+id);a.page.addView(button);}}
        String last=a.prefs.getString("lastRemoteRequest","");if(!last.isEmpty())a.action("Проверить последнюю команду",()->receipt(a,last,"Последняя команда"));
        a.text("Показаны только команды, разрешённые компьютером. Произвольные команды терминала не отправляются.");
    }
    static void refresh(MainActivity a){AtomicReference<JSONArray> result=new AtomicReference<>();a.job(()->result.set(call(a,"/v2/commands",new JSONObject()).getJSONArray("commands")),()->{if(a.tab==15){a.remoteCommands=result.get();a.show(15);}});}
    static void send(MainActivity a,String command,String title){String id=UUID.randomUUID().toString();a.prefs.edit().putString("lastRemoteRequest",id).apply();AtomicReference<JSONObject> response=new AtomicReference<>();a.job(()->response.set(call(a,"/v2/command",new JSONObject().put("id",id).put("command_id",command).put("expires_at",System.currentTimeMillis()/1000+60))),()->receipt(a,id,title));}
    static void receipt(MainActivity a,String id,String title){AtomicReference<JSONObject> result=new AtomicReference<>();a.job(()->result.set(call(a,"/v2/command/status",new JSONObject().put("id",id))),()->new AlertDialog.Builder(a).setTitle(title).setMessage(status(result.get())).setPositiveButton("Проверить результат",(d,w)->receipt(a,id,title)).setNegativeButton("Закрыть",null).show());}
}
