package com.diberlog.jarvis.mobile;

import android.content.Context;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import org.json.*;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class SyncDirectionTests {
    @Test public void pickerOffersThreeModesAndCancelLeavesChoiceUnchanged()throws Exception{
        android.app.Instrumentation i=androidx.test.platform.app.InstrumentationRegistry.getInstrumentation();
        com.diberlog.jarvis.mobile.MainActivity a=(MainActivity)i.startActivitySync(new android.content.Intent(i.getTargetContext(),MainActivity.class).addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK));
        String previous=a.prefs.getString("syncDirection","both");
        try{i.runOnMainSync(()->{a.show(12);a.chooseSyncDirection();});i.waitForIdleSync();assertTrue(a.choosingSync);
            android.view.accessibility.AccessibilityNodeInfo root=i.getUiAutomation().getRootInActiveWindow();assertNotNull(root);
            for(String label:new String[]{"ПК ↔ телефон · в обе стороны","Только ПК → телефон","Только телефон → ПК"})assertFalse(root.findAccessibilityNodeInfosByText(label).isEmpty());
            PolishTests.shot(i,a,"preview-sync-directions.png");
            assertTrue(root.findAccessibilityNodeInfosByText("Отмена").get(0).performAction(android.view.accessibility.AccessibilityNodeInfo.ACTION_CLICK));i.waitForIdleSync();assertFalse(a.choosingSync);assertEquals(previous,a.prefs.getString("syncDirection","both"));
        }finally{i.runOnMainSync(a::finish);}
    }
    Context client(String name,int id)throws Exception{Context c=SyncV2Tests.temporary(name);SyncJournal.provision(c,SyncV2Tests.group(1),String.format("%064x",id));JSONObject data=MobileStore.center(c);data.getJSONArray("notes").put(SyncV2Tests.note(name,"fictional-"+name));MobileStore.save(c,data);return c;}
    @Test public void pushOnlyAcknowledgesButKeepsUnreceivedCursorAndData()throws Exception{
        Context pc=client("pc",1),phone=client("phone",2);JSONObject pcOp=SyncJournal.outbound(pc).getJSONArray("operations").getJSONObject(0),phoneOp=SyncJournal.outbound(phone).getJSONArray("operations").getJSONObject(0);
        JSONArray items=new JSONArray().put(new JSONArray().put(1).put(pcOp)).put(new JSONArray().put(2).put(phoneOp));JSONObject response=SyncV2Tests.feed(items,2,new JSONArray().put(phoneOp.getString("op_id")));
        assertEquals(1,SyncClient.packet(phone,"to_pc").getJSONArray("operations").length());
        for(int n=0;n<10;n++)SyncJournal.receive(phone,response,false);
        assertEquals(1,MobileStore.center(phone).getJSONArray("notes").length());assertEquals("phone",MobileStore.center(phone).getJSONArray("notes").getJSONObject(0).getString("id"));
        assertEquals(0,SyncJournal.outbound(phone).getJSONArray("operations").length());assertEquals(0,SyncJournal.outbound(phone).getLong("cursor"));
        SyncJournal.receive(pc,SyncV2Tests.feed(items,2,new JSONArray()));assertEquals(2,MobileStore.center(pc).getJSONArray("notes").length());
        SyncJournal.receive(phone,response,true);assertEquals(2,MobileStore.center(phone).getJSONArray("notes").length());assertEquals(2,SyncJournal.outbound(phone).getLong("cursor"));
    }
    @Test public void pullOnlyDoesNotSendOrClearLocalQueue()throws Exception{
        Context pc=client("pc",3),phone=client("phone",4);JSONObject pcOp=SyncJournal.outbound(pc).getJSONArray("operations").getJSONObject(0);
        JSONObject request=SyncClient.packet(phone,"from_pc");assertEquals(0,request.getJSONArray("operations").length());assertFalse(request.getBoolean("recovering"));
        SyncJournal.receive(phone,SyncV2Tests.feed(new JSONArray().put(new JSONArray().put(1).put(pcOp)),1,new JSONArray()),true);
        assertEquals(2,MobileStore.center(phone).getJSONArray("notes").length());assertEquals(1,SyncJournal.outbound(phone).getJSONArray("operations").length());assertEquals(1,SyncClient.packet(phone,"both").getJSONArray("operations").length());
        try{SyncClient.packet(phone,"invalid");fail("Invalid direction must not start two-way exchange");}catch(java.io.IOException expected){}
        assertEquals(1,SyncJournal.outbound(phone).getJSONArray("operations").length());
    }
}
