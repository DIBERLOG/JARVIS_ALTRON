package com.diberlog.jarvis.mobile;

import android.app.*;
import android.content.*;
import android.os.Build;
import org.json.*;
import java.time.*;

public final class ReminderReceiver extends BroadcastReceiver {
    static final String CHANNEL="jarvis-reminders";
    static void channels(Context c){NotificationManager n=c.getSystemService(NotificationManager.class);n.createNotificationChannel(new NotificationChannel(CHANNEL,"Напоминания и таймер",NotificationManager.IMPORTANCE_HIGH));}
    static long millis(String value){try{return Instant.parse(value).toEpochMilli();}catch(Exception e){return LocalDateTime.parse(value).atZone(ZoneId.systemDefault()).toInstant().toEpochMilli();}}
    static PendingIntent pending(Context c,String id,String title){Intent i=new Intent(c,ReminderReceiver.class).setAction("jarvis."+id).putExtra("id",id).putExtra("title",title);return PendingIntent.getBroadcast(c,id.hashCode(),i,PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);}
    static void alarm(Context c,String id,String title,long when){if(when<=System.currentTimeMillis())return;AlarmManager a=c.getSystemService(AlarmManager.class);PendingIntent p=pending(c,id,title);if(Build.VERSION.SDK_INT<31||a.canScheduleExactAlarms())a.setExactAndAllowWhileIdle(AlarmManager.RTC_WAKEUP,when,p);else a.setAndAllowWhileIdle(AlarmManager.RTC_WAKEUP,when,p);}
    static void cancel(Context c,String id){c.getSystemService(AlarmManager.class).cancel(pending(c,id,""));}
    static void schedule(Context c){channels(c);try{
        android.content.SharedPreferences prefs=MobileStore.prefs(c);JSONArray previous=new JSONArray(prefs.getString("scheduled","[]"));for(int i=0;i<previous.length();i++)cancel(c,previous.getString(i));JSONArray ids=new JSONArray();JSONObject d=MobileStore.center(c);
        JSONArray rs=d.getJSONArray("reminders");for(int i=0;i<rs.length();i++){JSONObject r=rs.getJSONObject(i);if(!r.optBoolean("done")){String id="reminder-"+r.getString("id");long when=millis(r.getString("dueAt"));if(when>System.currentTimeMillis()){alarm(c,id,r.getString("title"),when);ids.put(id);}}}
        JSONArray bs=d.getJSONArray("birthdays");for(int i=0;i<bs.length();i++){JSONObject b=bs.getJSONObject(i);LocalDate today=LocalDate.now();int year=Math.max(today.getYear(),b.optInt("year",today.getYear()));LocalDate date=birthday(year,b.getInt("month"),b.getInt("day"));if(date.atTime(9,0).atZone(ZoneId.systemDefault()).toInstant().toEpochMilli()<=System.currentTimeMillis())date=birthday(year+1,b.getInt("month"),b.getInt("day"));String id="birthday-"+b.getString("id");alarm(c,id,"День рождения: "+b.getString("name"),date.atTime(9,0).atZone(ZoneId.systemDefault()).toInstant().toEpochMilli());ids.put(id);}
        long timer=prefs.getLong("timerEnd",0);if(timer>System.currentTimeMillis())alarm(c,"timer","Таймер завершён",timer);
        prefs.edit().putString("scheduled",ids.toString()).apply();
    }catch(Exception ignored){/* Preserved data; UI reports invalid entries when opened. */}}
    static LocalDate birthday(int year,int month,int day){try{return LocalDate.of(year,month,day);}catch(DateTimeException e){if(month==2&&day==29)return LocalDate.of(year,3,1);throw e;}}
    @Override public void onReceive(Context c,Intent intent){channels(c);String id=intent.getStringExtra("id");if(id==null)return;
        if("timer".equals(id))MobileStore.prefs(c).edit().putLong("timerEnd",0).apply();
        Intent open=new Intent(c,MainActivity.class);PendingIntent p=PendingIntent.getActivity(c,0,open,PendingIntent.FLAG_UPDATE_CURRENT|PendingIntent.FLAG_IMMUTABLE);
        Notification note=new Notification.Builder(c,CHANNEL).setSmallIcon(android.R.drawable.ic_dialog_info).setContentTitle("JARVIS").setContentText(intent.getStringExtra("title")).setAutoCancel(true).setContentIntent(p).build();
        try{c.getSystemService(NotificationManager.class).notify(id.hashCode(),note);}catch(SecurityException ignored){}
        if(id.startsWith("birthday-"))schedule(c);
    }
}
