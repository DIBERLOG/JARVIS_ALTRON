package com.diberlog.jarvis.mobile;
import android.content.Context;
import androidx.work.*;
import java.util.concurrent.TimeUnit;
public final class SyncWorker extends Worker {
    public SyncWorker(Context context,WorkerParameters params){super(context,params);}
    @Override public Result doWork(){if(!MobileStore.prefs(getApplicationContext()).getBoolean("autoSync",false))return Result.success();try{SyncClient.sync(getApplicationContext());return Result.success();}catch(Exception e){MobileStore.prefs(getApplicationContext()).edit().putString("lastSyncError","Автообмен не выполнен. Проверьте Wi-Fi, запущенный JARVIS на ПК и сопряжение.").apply();return Result.success();}}
    static void configure(Context c){if(MobileStore.prefs(c).getBoolean("autoSync",false)){Constraints constraints=new Constraints.Builder().setRequiredNetworkType(NetworkType.UNMETERED).build();PeriodicWorkRequest request=new PeriodicWorkRequest.Builder(SyncWorker.class,15,TimeUnit.MINUTES).setConstraints(constraints).build();WorkManager.getInstance(c).enqueueUniquePeriodicWork("jarvis-local-sync",ExistingPeriodicWorkPolicy.UPDATE,request);}else WorkManager.getInstance(c).cancelUniqueWork("jarvis-local-sync");}
}
