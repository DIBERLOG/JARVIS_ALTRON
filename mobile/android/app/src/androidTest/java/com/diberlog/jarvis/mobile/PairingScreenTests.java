package com.diberlog.jarvis.mobile;

import android.app.*;
import android.content.*;
import android.view.*;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import org.json.JSONObject;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class PairingScreenTests {
    @Test public void pendingAndConnectedComputerShowDifferentActions()throws Exception{
        Instrumentation i=InstrumentationRegistry.getInstrumentation();MainActivity a=(MainActivity)i.startActivitySync(new Intent(i.getTargetContext(),MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));JSONObject old=PrivateStorage.pairing(a);boolean auto=a.prefs.getBoolean("autoSync",false);
        try{a.prefs.edit().putBoolean("autoSync",false).commit();i.runOnMainSync(()->a.foreground=false);
            PrivateStorage.pairing(a,new JSONObject().put("host","192.168.1.42").put("port",47832).put("pendingId","fictional-request").put("code","123456"));
            i.runOnMainSync(()->{a.show(12);assertNotNull(a.page.findViewWithTag("paired-computer"));assertNotNull(a.page.findViewWithTag("pair-finish"));assertNull(a.page.findViewWithTag("sync-now"));});PolishTests.shot(i,a,"preview-pair-pending.png");
            PrivateStorage.pairing(a,new JSONObject().put("host","192.168.1.42").put("port",47832).put("token","fictional-token"));
            i.runOnMainSync(()->{a.show(12);assertNotNull(a.page.findViewWithTag("paired-computer"));assertNotNull(a.page.findViewWithTag("sync-now"));assertNull(a.page.findViewWithTag("pair-finish"));a.pairingSuccess();});PolishTests.shot(i,a,"preview-pair-success.png");
            i.runOnMainSync(()->{View decor=a.getWindow().getDecorView();assertTrue(decor.isShown());});
        }finally{PrivateStorage.pairing(a,old);a.prefs.edit().putBoolean("autoSync",auto).commit();i.runOnMainSync(a::finish);}
    }
}
