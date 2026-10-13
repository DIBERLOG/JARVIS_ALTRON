package com.diberlog.jarvis.mobile;
import android.content.*;
public final class BootReceiver extends BroadcastReceiver { @Override public void onReceive(Context c,Intent i){if(java.util.Arrays.asList(Intent.ACTION_BOOT_COMPLETED,Intent.ACTION_MY_PACKAGE_REPLACED,Intent.ACTION_TIME_CHANGED,Intent.ACTION_TIMEZONE_CHANGED,"android.app.action.SCHEDULE_EXACT_ALARM_PERMISSION_STATE_CHANGED").contains(i.getAction()))ReminderReceiver.schedule(c);} }
