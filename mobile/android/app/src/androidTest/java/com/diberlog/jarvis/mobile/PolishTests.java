package com.diberlog.jarvis.mobile;

import android.app.Instrumentation;
import android.content.Intent;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import org.json.*;
import org.junit.Test;
import org.junit.runner.RunWith;
import java.io.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicReference;
import java.util.zip.*;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class PolishTests {
    @Test public void deepseekStreamKeepsAnswerAndRejectsIncompleteOutput()throws Exception{String events="data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"internal fictional reasoning\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"Готовый документ\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n";assertEquals("Готовый документ",ChatClient.readStream(new ByteArrayInputStream(events.getBytes(java.nio.charset.StandardCharsets.UTF_8))));try{ChatClient.readStream(new ByteArrayInputStream(events.replace("data: [DONE]","").getBytes(java.nio.charset.StandardCharsets.UTF_8)));fail("Incomplete output accepted");}catch(IOException expected){}}
    @Test public void wordDocumentRoundTripsUnicodeAndXmlCharacters()throws Exception{String text="Заголовок & <пример>\nВторая строка 🙂\n";ByteArrayOutputStream out=new ByteArrayOutputStream();ChatDocuments.writeDocx(out,text);assertEquals(text+"\n",ChatDocuments.readDocx(new ByteArrayInputStream(out.toByteArray())));}
    @Test public void wordDocumentRejectsExternalEntities()throws Exception{ByteArrayOutputStream out=new ByteArrayOutputStream();try(ZipOutputStream zip=new ZipOutputStream(out)){ChatDocuments.entry(zip,"word/document.xml","<!DOCTYPE x [<!ENTITY stolen SYSTEM 'file:///fictional-secret'>]><x>&stolen;</x>");}try{ChatDocuments.readDocx(new ByteArrayInputStream(out.toByteArray()));fail("External entity accepted");}catch(IOException expected){assertTrue(expected.getMessage().contains("сущности"));}}
    @Test public void weatherAdviceUsesFeelsLikeRainAndWind()throws Exception{JSONObject data=new JSONObject().put("current",new JSONObject().put("apparent_temperature",3).put("wind_speed_10m",9)).put("days",new JSONArray().put(new JSONObject().put("rain",85)));String advice=MobilePolish.clothing(data);assertTrue(advice.contains("Куртка"));assertTrue(advice.contains("зонт"));assertTrue(advice.contains("ветровка"));}
    @Test public void qrPairingSupportsPrivateLanAndRejectsPublicAddress()throws Exception{String qr="jarvis-pair://v2?host=192.168.1.12&port=47832&fingerprint="+new String(new char[64]).replace('\0','a')+"&secret="+new String(new char[64]).replace('\0','b');assertEquals("192.168.1.12",SyncClient.parseQr(qr).getString("host"));try{SyncClient.parseQr(qr.replace("192.168.1.12","8.8.8.8"));fail("Public host accepted");}catch(IOException expected){}}
    @Test public void mobileTrainingImageLoadsAndNotesStartCompact()throws Exception{
        Instrumentation i=InstrumentationRegistry.getInstrumentation();MainActivity activity=(MainActivity)i.startActivitySync(new Intent(i.getTargetContext(),MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));try{
            i.runOnMainSync(()->activity.show(10));assertTrue(waitJs(i,activity,"Boolean(document.querySelector('.canvas img')?.complete && document.querySelector('.canvas img')?.naturalWidth>0)"));shot(i,activity,"preview-training.png");
            i.runOnMainSync(()->activity.show(3));assertTrue(waitJs(i,activity,"Boolean(document.querySelector('.mobile-compact') && !document.querySelector('.mobile-compact').classList.contains('expanded'))"));shot(i,activity,"preview-notes.png");
            i.runOnMainSync(()->activity.show(9));assertTrue(waitJs(i,activity,"Boolean(document.querySelector('.habits'))"));shot(i,activity,"preview-habits.png");
        }finally{i.runOnMainSync(activity::finish);}
    }
    static void shot(Instrumentation i,MainActivity activity,String name)throws Exception{i.waitForIdleSync();Thread.sleep(400);android.graphics.Bitmap bitmap=i.getUiAutomation().takeScreenshot();assertNotNull(bitmap);try(FileOutputStream out=new FileOutputStream(new File(activity.getExternalFilesDir(null),name))){assertTrue(bitmap.compress(android.graphics.Bitmap.CompressFormat.PNG,100,out));}bitmap.recycle();}
    @Test public void selectedPhotoBecomesEmbeddedNoteImageWithoutFileAccess()throws Exception{
        Instrumentation i=InstrumentationRegistry.getInstrumentation();MainActivity activity=(MainActivity)i.startActivitySync(new Intent(i.getTargetContext(),MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));android.net.Uri photo=null;try{
            i.runOnMainSync(()->activity.show(3));assertTrue(waitJs(i,activity,"Boolean(document.querySelector('.mobile-compact'))"));
            android.content.ContentValues values=new android.content.ContentValues();values.put(android.provider.MediaStore.Images.Media.DISPLAY_NAME,"jarvis-fictional-"+java.util.UUID.randomUUID()+".png");values.put(android.provider.MediaStore.Images.Media.MIME_TYPE,"image/png");photo=activity.getContentResolver().insert(android.provider.MediaStore.Images.Media.EXTERNAL_CONTENT_URI,values);assertNotNull(photo);
            android.graphics.Bitmap fake=android.graphics.Bitmap.createBitmap(16,16,android.graphics.Bitmap.Config.ARGB_8888);fake.eraseColor(android.graphics.Color.CYAN);try(OutputStream out=activity.getContentResolver().openOutputStream(photo)){assertTrue(fake.compress(android.graphics.Bitmap.CompressFormat.PNG,100,out));}fake.recycle();
            activity.centerPage.photoResult(photo);assertTrue(waitJs(i,activity,"Boolean(document.querySelector('.ProseMirror img')?.complete && document.querySelector('.ProseMirror img')?.naturalWidth>0)"));
            i.runOnMainSync(()->{assertFalse(activity.centerPage.view.getSettings().getAllowFileAccess());assertFalse(activity.centerPage.view.getSettings().getAllowContentAccess());assertTrue(activity.centerPage.dirty);});
        }finally{if(photo!=null)activity.getContentResolver().delete(photo,null,null);i.runOnMainSync(activity::finish);}
    }
    static boolean waitJs(Instrumentation i,MainActivity activity,String js)throws Exception{for(int n=0;n<20;n++){CountDownLatch latch=new CountDownLatch(1);AtomicReference<String> result=new AtomicReference<>();i.runOnMainSync(()->activity.centerPage.view.evaluateJavascript(js,value->{result.set(value);latch.countDown();}));if(!latch.await(2,TimeUnit.SECONDS))return false;if("true".equals(result.get()))return true;Thread.sleep(250);}return false;}
    @Test public void rejectedNoteSaveDoesNotPoisonFollowingSave()throws Exception{
        Instrumentation i=InstrumentationRegistry.getInstrumentation();MainActivity activity=(MainActivity)i.startActivitySync(new Intent(i.getTargetContext(),MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));String good="fixture-good-"+java.util.UUID.randomUUID();try{
            i.runOnMainSync(()->activity.show(3));assertTrue(waitJs(i,activity,"Boolean(document.querySelector('.mobile-compact'))"));String original=activity.centerPage.base.toString();char[] huge=new char[4*1024*1024];java.util.Arrays.fill(huge,'x');JSONObject bad=new JSONObject().put("id","fixture-too-large").put("title","Вымышленная слишком большая заметка").put("type","text").put("text",new String(huge));
            assertFalse(new JSONObject(activity.centerPage.save(new JSONObject().put("note",bad).toString())).getBoolean("ok"));assertEquals(original,activity.centerPage.base.toString());
            JSONObject valid=new JSONObject().put("id",good).put("title","Вымышленная корректная заметка").put("type","text").put("text","Только тест").put("updatedAt",MobileStore.now());assertTrue(new JSONObject(activity.centerPage.save(new JSONObject().put("note",valid).toString())).getBoolean("ok"));
        }finally{JSONObject center=MobileStore.center(activity);JSONArray notes=center.getJSONArray("notes");for(int n=notes.length()-1;n>=0;n--)if(notes.getJSONObject(n).getString("id").equals(good))notes.remove(n);MobileStore.save(activity,center);i.runOnMainSync(activity::finish);}
    }
}
