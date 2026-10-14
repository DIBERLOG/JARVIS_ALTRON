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
            i.runOnMainSync(()->{a.foreground=true;a.pairChecks=40;a.checkPairing(false);assertFalse(a.pairError.isEmpty());a.foreground=false;});
            JSONObject pending=PrivateStorage.pairing(a),approval=new JSONObject().put("version",2).put("token","fictional-token").put("deviceId","fictional-device");pending.put("secret","fictional-secret").put("approved",approval);PrivateStorage.pairing(a,pending);
            String before=SyncJournal.snapshot(a).toString();assertTrue(SyncClient.checkApproval(a));assertFalse(PrivateStorage.pairing(a).has("token"));assertEquals(before,SyncJournal.snapshot(a).toString());
            i.runOnMainSync(()->{a.pairError="";a.show(12);assertNotNull(a.page.findViewWithTag("pair-confirm"));assertNull(a.page.findViewWithTag("pair-finish"));assertNull(a.page.findViewWithTag("sync-now"));});PolishTests.shot(i,a,"preview-pair-confirm.png");
            JSONObject completed=SyncClient.completedPairing(pending,approval);
            for(String key:new String[]{"secret","pendingId","code","approved"})assertFalse(completed.has(key));assertTrue(pending.has("code"));
            try{SyncClient.completedPairing(pending,new JSONObject().put("version",2));fail("Incomplete approval accepted");}catch(java.io.IOException expected){}
            PrivateStorage.pairing(a,completed);
            i.runOnMainSync(()->{a.pairError="";a.foreground=true;a.checkPairing(false);assertEquals("",a.pairError);a.foreground=false;a.show(12);assertNotNull(a.page.findViewWithTag("paired-computer"));assertNotNull(a.page.findViewWithTag("sync-now"));assertNull(a.page.findViewWithTag("pair-finish"));assertNull(a.page.findViewWithTag("pair-confirm"));a.pairingSuccess();});PolishTests.shot(i,a,"preview-pair-success.png");
            i.runOnMainSync(()->{View decor=a.getWindow().getDecorView();assertTrue(decor.isShown());});
        }finally{PrivateStorage.pairing(a,old);a.prefs.edit().putBoolean("autoSync",auto).commit();i.runOnMainSync(a::finish);}
    }
}
