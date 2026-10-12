package com.diberlog.jarvis.mobile;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import android.content.*;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.json.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import javax.crypto.SecretKey;
import javax.crypto.spec.SecretKeySpec;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class SyncV2Tests {
    static JSONObject group(int epoch)throws Exception{JSONArray first=new JSONArray(),second=new JSONArray();for(int i=0;i<32;i++){first.put(42);second.put(77);}JSONObject keys=new JSONObject().put("1",first);if(epoch==2)keys.put("2",second);return new JSONObject().put("version",2).put("group_id",String.format("%064x",15)).put("epoch",epoch).put("keys",keys);}
    static Context temporary(String prefix){Context base=InstrumentationRegistry.getInstrumentation().getTargetContext();String name="v2-test-"+prefix+"-"+java.util.UUID.randomUUID();File directory=new File(base.getCacheDir(),name);assertTrue(directory.mkdirs());return new ContextWrapper(base){@Override public File getFilesDir(){return directory;}@Override public SharedPreferences getSharedPreferences(String key,int mode){return base.getSharedPreferences(name+key,mode);}};}
    static JSONObject note(String id,String text)throws Exception{return new JSONObject().put("id",id).put("title","fictional").put("text",text).put("updatedAt","2026-10-12T12:00:00Z");}
    static JSONObject feed(JSONArray items,long cursor,JSONArray acks)throws Exception{return new JSONObject().put("version",2).put("group_id",group(1).getString("group_id")).put("epoch",1).put("items",items).put("cursor",cursor).put("acknowledged",acks);}
    @Test public void threeRealStoresOfflineRetryDeleteConflictRollbackAndRecovery()throws Exception{
        Context[] clients={temporary("a"),temporary("b"),temporary("c")};JSONArray all=new JSONArray();
        for(int i=0;i<3;i++){SyncJournal.provision(clients[i],group(1),String.format("%064x",i));JSONObject center=MobileStore.empty();center.getJSONArray("notes").put(note("n"+i,"fictional-private-offline-"+i));MobileStore.save(clients[i],center);JSONObject operation=SyncJournal.outbound(clients[i]).getJSONArray("operations").getJSONObject(0);all.put(new JSONArray().put(i+1).put(operation));}
        for(Context c:clients){JSONArray acks=new JSONArray().put(SyncJournal.outbound(c).getJSONArray("operations").getJSONObject(0).getString("op_id"));for(int i=0;i<10;i++)SyncJournal.receive(c,feed(all,3,acks));assertEquals(3,MobileStore.center(c).getJSONArray("notes").length());assertEquals(0,SyncJournal.outbound(c).getJSONArray("operations").length());}
        JSONObject deletion=MobileStore.center(clients[0]);deletion.getJSONArray("notes").remove(0);MobileStore.save(clients[0],deletion);
        JSONObject edited=MobileStore.center(clients[1]);edited.getJSONArray("notes").getJSONObject(0).put("text","concurrent-edit");MobileStore.save(clients[1],edited);
        JSONArray changes=new JSONArray().put(new JSONArray().put(4).put(SyncJournal.outbound(clients[0]).getJSONArray("operations").getJSONObject(0))).put(new JSONArray().put(5).put(SyncJournal.outbound(clients[1]).getJSONArray("operations").getJSONObject(0)));
        for(Context c:clients){SyncJournal.receive(c,feed(changes,5,new JSONArray()));assertEquals(2,MobileStore.center(c).getJSONArray("notes").length());assertEquals(1,SyncJournal.conflicts(c).length());}
        JSONObject conflict=SyncJournal.conflicts(clients[2]).getJSONObject(0);JSONArray parents=new JSONArray();for(int i=0;i<2;i++)parents.put(conflict.getJSONArray("versions").getJSONObject(i).getString("op_id"));
        SyncJournal.resolve(clients[2],"notes","n0",parents,note("n0","chosen-explicitly"));JSONObject resolution=SyncJournal.outbound(clients[2]).getJSONArray("operations").getJSONObject(0);JSONArray finalFeed=new JSONArray().put(new JSONArray().put(6).put(resolution));
        for(Context c:clients){SyncJournal.receive(c,feed(finalFeed,6,new JSONArray()));assertEquals(0,SyncJournal.conflicts(c).length());assertEquals(3,MobileStore.center(c).getJSONArray("notes").length());}
        JSONObject before=SyncJournal.snapshot(clients[2]);JSONObject broken=new JSONObject(resolution.toString()).put("entity_id","tampered");
        try{SyncJournal.receive(clients[2],feed(new JSONArray().put(new JSONArray().put(7).put(broken)),7,new JSONArray().put(resolution.getString("op_id"))));fail("Corruption accepted");}catch(javax.crypto.AEADBadTagException expected){}
        assertTrue(MobileStore.same(before,SyncJournal.snapshot(clients[2])));assertEquals(6,SyncJournal.outbound(clients[2]).getLong("cursor"));assertEquals(1,SyncJournal.outbound(clients[2]).getJSONArray("operations").length());
        JSONObject backup=SyncJournal.exportRecovery(clients[2],"fictional recovery master".toCharArray());assertFalse(backup.toString().contains("fictional-private"));Context fresh=temporary("restored");
        try{SyncJournal.restoreRecovery(fresh,backup,"incorrect password".toCharArray());fail("Wrong recovery password accepted");}catch(javax.crypto.AEADBadTagException expected){}
        assertFalse(new File(fresh.getFilesDir(),"sync-v2.sqlite").exists());SyncJournal.restoreRecovery(fresh,backup,"fictional recovery master".toCharArray());assertTrue(MobileStore.same(MobileStore.center(clients[2]),MobileStore.center(fresh)));assertTrue(SyncJournal.outbound(fresh).getBoolean("recovering"));
    }
    @Test public void rotationRewrapsDurableOfflineQueueWithoutChangingOperationId()throws Exception{
        Context c=temporary("rotation");String device=String.format("%064x",9);SyncJournal.provision(c,group(1),device);JSONObject center=MobileStore.empty();center.getJSONArray("notes").put(note("rotate","fictional-future"));MobileStore.save(c,center);JSONObject before=SyncJournal.outbound(c).getJSONArray("operations").getJSONObject(0);
        SyncJournal.provision(c,group(2),device);SyncJournal.rewrap(c,new JSONArray().put(before.getString("op_id")));JSONObject after=SyncJournal.outbound(c).getJSONArray("operations").getJSONObject(0);assertEquals(before.getString("op_id"),after.getString("op_id"));assertEquals(2,after.getInt("key_version"));
        try{SyncJournal.open(after.getJSONObject("sealed"),SyncJournal.aad(after),SyncJournal.groupKey(group(1),1));fail("Revoked key read future operation");}catch(javax.crypto.AEADBadTagException expected){}
        SyncJournal.receive(c,new JSONObject().put("version",2).put("group_id",group(2).getString("group_id")).put("epoch",2).put("items",new JSONArray().put(new JSONArray().put(1).put(after))).put("cursor",1).put("acknowledged",new JSONArray().put(after.getString("op_id"))));assertEquals(0,SyncJournal.outbound(c).getJSONArray("operations").length());
    }
    @Test public void migrationIsEncryptedAndRetainsOriginalCopy()throws Exception{
        Context c=temporary("migration");JSONObject center=MobileStore.empty();center.getJSONArray("notes").put(note("legacy","fictional-legacy-secret"));MobileStore.prefs(c).edit().putString("center",center.toString()).commit();assertTrue(MobileStore.same(center,MobileStore.center(c)));assertEquals(center.toString(),MobileStore.prefs(c).getString("center",""));
        for(String file:new String[]{"sync-v2.sqlite","sync-v2-migration-encrypted.json"}){String raw=new String(java.nio.file.Files.readAllBytes(new File(c.getFilesDir(),file).toPath()),StandardCharsets.ISO_8859_1);assertFalse(raw.contains("fictional-legacy-secret"));}
    }
    @Test public void rustAndAndroidOperationsAndUnicodeRecoveryAreCompatible()throws Exception{
        Context test=InstrumentationRegistry.getInstrumentation().getContext();JSONObject fixture;try(InputStream in=test.getAssets().open("desktop-sync-v2-fixture.json")){fixture=new JSONObject(PublicData.read(in,65536));}
        JSONObject op=fixture.getJSONObject("operation");byte[] key=new byte[32];for(int i=0;i<32;i++)key[i]=(byte)fixture.getJSONArray("key").getInt(i);SecretKey aes=new SecretKeySpec(key,"AES");assertTrue(MobileStore.same(fixture.get("value"),SyncJournal.open(op.getJSONObject("sealed"),SyncJournal.aad(op),aes)));
        assertEquals("Совместимость",SyncJournal.recoveryOpen(fixture.getJSONObject("recovery"),fixture.getString("password").toCharArray()).getString("fictional"));
        op=new JSONObject(op.toString());op.put("op_id",SyncJournal.random());op.put("sealed",SyncJournal.seal(fixture.get("value"),SyncJournal.aad(op),aes));
        Context app=InstrumentationRegistry.getInstrumentation().getTargetContext();PrivateStorage.write(new File(app.getFilesDir(),"android-sync-v2-fixture.json"),new JSONObject().put("operation",op).put("value",fixture.get("value")).toString());
    }
}
