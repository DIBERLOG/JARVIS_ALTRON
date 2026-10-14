package com.diberlog.jarvis.mobile;

import android.content.Context;
import android.os.Build;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import org.json.*;
import org.junit.*;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

/** Explicit upgrade fixture for our dedicated emulator, never a personal phone. */
@RunWith(AndroidJUnit4.class)
public class StartupTests {
    @Test public void releaseUpgradeKeepsEncryptedProfile()throws Exception {
        String mode=InstrumentationRegistry.getArguments().getString("startupUpgrade","");
        Assume.assumeTrue(mode.equals("seed")||mode.equals("verify"));
        assertTrue("Dedicated emulator only",Build.MODEL.startsWith("sdk_gphone")||Build.FINGERPRINT.contains("generic"));
        Context c=InstrumentationRegistry.getInstrumentation().getTargetContext();
        if(mode.equals("seed")) {
            String id=MobileStore.id();JSONObject center=MobileStore.center(c);
            center.getJSONArray("notes").put(new JSONObject().put("id",id).put("title","Вымышленная заметка обновления").put("text","Тестовые данные, не личная запись").put("type","text").put("format","plain").put("updatedAt",MobileStore.now()));MobileStore.save(c,center);
            JSONArray chats=MobileStore.chats(c);chats.put(new JSONObject().put("id",id).put("title","Вымышленный чат обновления").put("updatedAt",MobileStore.now()).put("messages",new JSONArray().put(new JSONObject().put("role","user").put("content","Вымышленный вопрос"))));MobileStore.saveChats(c,chats);
            ChatDrafts.text(c,id,"Вымышленный черновик");ChatDrafts.append(c,id,"fixture.txt","Вымышленный документ");
            assertTrue(MobileStore.prefs(c).edit().putString("startupUpgradeFixture",id).putString("appearance","light").putString("lastChatId",id).commit());
        } else {
            String id=MobileStore.prefs(c).getString("startupUpgradeFixture","");assertFalse(id.isEmpty());boolean note=false,chat=false;
            JSONArray notes=MobileStore.center(c).getJSONArray("notes");for(int n=0;n<notes.length();n++)if(notes.getJSONObject(n).getString("id").equals(id)){assertEquals("Вымышленная заметка обновления",notes.getJSONObject(n).getString("title"));note=true;}
            JSONArray chats=MobileStore.chats(c);for(int n=0;n<chats.length();n++)if(chats.getJSONObject(n).getString("id").equals(id)){assertEquals("Вымышленный вопрос",chats.getJSONObject(n).getJSONArray("messages").getJSONObject(0).getString("content"));chat=true;}
            assertTrue(note);assertTrue(chat);assertEquals("light",MobileStore.prefs(c).getString("appearance",""));assertEquals("Вымышленный черновик",ChatDrafts.read(c,id).getString("text"));assertEquals("Вымышленный документ",ChatDrafts.read(c,id).getJSONArray("attachments").getJSONObject(0).getString("text"));
        }
    }
}
