package com.diberlog.jarvis.mobile;

import android.app.*;
import android.content.*;
import android.graphics.*;
import android.net.NetworkCapabilities;
import android.view.*;
import android.widget.*;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import com.google.zxing.*;
import com.google.zxing.common.*;
import org.json.*;
import org.junit.Test;
import org.junit.runner.RunWith;
import java.io.*;
import java.util.concurrent.*;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class NewsScannerTests {
    final Instrumentation i=InstrumentationRegistry.getInstrumentation();
    MainActivity open(){return (MainActivity)i.startActivitySync(new Intent(i.getTargetContext(),MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));}
    boolean text(View view,String value){if(view instanceof TextView&&((TextView)view).getText().toString().contains(value))return true;if(view instanceof ViewGroup){ViewGroup group=(ViewGroup)view;for(int n=0;n<group.getChildCount();n++)if(text(group.getChildAt(n),value))return true;}return false;}
    @Test public void categoriesChangeDirectlyHideStaleFeedAndScrollToLastTopic()throws Exception{
        MainActivity a=open();String topic=a.prefs.getString("newsTopic","all"),region=a.prefs.getString("newsRegion","ru"),source=a.prefs.getString("newsDomain","");Object old=SyncJournal.snapshot(a).opt("news_data");
        try{a.prefs.edit().putString("newsTopic","all").putString("newsRegion","ru").putString("newsDomain","").putBoolean("newsCustom",false).commit();String selection=new MobileFeatures(a).newsSelectionKey();SyncJournal.save(a,"news_data",new JSONObject().put("selection",selection).put("items",new JSONArray().put(new JSONObject().put("title","Вымышленное главное").put("source","Тестовый источник").put("summary","Тестовая публикация").put("url","https://example.com/fixture"))).put("updatedAt",MobileStore.now()).put("source","Главное"));
            i.runOnMainSync(()->{a.publicBusy=true;a.publicOrigin=2;a.newsAttemptSelection=selection;a.show(2);assertTrue(text(a.page,"Вымышленное главное"));});PolishTests.shot(i,a,"preview-news-layout.png");i.runOnMainSync(()->{assertTrue(a.page.findViewWithTag("news-category-1").performClick());assertEquals("ai",a.prefs.getString("newsTopic",""));assertFalse(text(a.page,"Вымышленное главное"));assertTrue(a.page.findViewWithTag("news-category-1").isSelected());assertNotNull(a.shell.findViewWithTag("loading-indicator"));assertTrue(a.newsRefreshPending);a.page.findViewWithTag("news-category-7").performClick();assertEquals("space",a.prefs.getString("newsTopic",""));});
            i.waitForIdleSync();i.runOnMainSync(()->assertTrue(((HorizontalScrollView)a.page.findViewWithTag("news-categories")).getScrollX()>0));PolishTests.shot(i,a,"preview-news-categories.png");
        }finally{i.runOnMainSync(()->{a.publicBusy=false;a.show(0);});a.prefs.edit().putString("newsTopic",topic).putString("newsRegion",region).putString("newsDomain",source).apply();SyncJournal.save(a,"news_data",old==null?JSONObject.NULL:old);i.runOnMainSync(a::finish);}
    }
    @Test public void loadingRingFollowsItsScreenAndClearsAfterSuccess()throws Exception{
        MainActivity a=open();CountDownLatch release=new CountDownLatch(1),done=new CountDownLatch(1);
        try{i.runOnMainSync(()->{a.show(8);a.publicJob(()->release.await(5,TimeUnit.SECONDS),done::countDown);assertNotNull(a.shell.findViewWithTag("loading-indicator"));a.show(1);assertNull(a.shell.findViewWithTag("loading-indicator"));a.show(8);assertNotNull(a.shell.findViewWithTag("loading-indicator"));});release.countDown();assertTrue(done.await(3,TimeUnit.SECONDS));i.runOnMainSync(()->assertNull(a.shell.findViewWithTag("loading-indicator")));}finally{release.countDown();i.runOnMainSync(a::finish);}
    }
    NetworkCapabilities caps(int... types){android.net.NetworkRequest.Builder builder=new android.net.NetworkRequest.Builder();for(int type:types)builder.addTransportType(type);android.os.Parcel parcel=android.os.Parcel.obtain();try{builder.build().writeToParcel(parcel,0);parcel.setDataPosition(0);return NetworkCapabilities.CREATOR.createFromParcel(parcel);}finally{parcel.recycle();}}
    @Test public void vpnOverlayIsNotSelectedAsPhysicalWifi(){assertTrue(SyncClient.physicalWifi(caps(NetworkCapabilities.TRANSPORT_WIFI)));assertFalse(SyncClient.physicalWifi(caps(NetworkCapabilities.TRANSPORT_WIFI,NetworkCapabilities.TRANSPORT_VPN)));assertFalse(SyncClient.physicalWifi(caps(NetworkCapabilities.TRANSPORT_CELLULAR)));assertFalse(SyncClient.physicalWifi(null));}
    @Test public void vpnBindRefusalAllowsFallbackButPostAndTlsErrorsDoNot(){
        assertTrue(SyncClient.bindingDenied(new IOException("Binding socket to network 289 failed: EPERM (Operation not permitted)")));
        assertTrue(SyncClient.bindingDenied(new IOException("connection",new java.net.SocketException("Binding socket to network 42 failed: EACCES"))));
        assertFalse(SyncClient.bindingDenied(new javax.net.ssl.SSLHandshakeException("Сертификат компьютера изменился")));
        assertFalse(SyncClient.bindingDenied(new java.net.SocketTimeoutException("Read timed out")));
        assertFalse(SyncClient.bindingDenied(new IOException("POST response failed: EPERM")));
        assertFalse(SyncClient.bindingDenied(new IOException("read failed",new android.system.ErrnoException("read",android.system.OsConstants.EPERM))));
    }
    @Test public void qrDecoderRoundTripKeepsPairingValidation()throws Exception{String raw="jarvis-pair://v2?host=192.168.1.42&port=47832&fingerprint="+String.format("%064x",1)+"&secret="+String.format("%064x",2);BitMatrix matrix=new com.google.zxing.qrcode.QRCodeWriter().encode(raw,BarcodeFormat.QR_CODE,480,480);int[] pixels=new int[480*480];for(int y=0;y<480;y++)for(int x=0;x<480;x++)pixels[y*480+x]=matrix.get(x,y)?Color.BLACK:Color.WHITE;Result decoded=new com.google.zxing.qrcode.QRCodeReader().decode(new BinaryBitmap(new HybridBinarizer(new RGBLuminanceSource(480,480,pixels))));assertEquals("192.168.1.42",SyncClient.parseQr(decoded.getText()).getString("host"));try{SyncClient.parseQr("https://example.com");fail("Not a pairing QR");}catch(Exception expected){}}
    @Test public void customScannerShowsCaptureFrameAndCloseControl()throws Exception{
        QrScannerActivity scanner=(QrScannerActivity)i.startActivitySync(new Intent(i.getTargetContext(),QrScannerActivity.class).putExtra("SCAN_FORMATS","QR_CODE").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        try{i.waitForIdleSync();i.runOnMainSync(()->{assertNotNull(scanner.findViewById(android.R.id.content).findViewWithTag("qr-capture-guide"));assertTrue(text(scanner.findViewById(android.R.id.content),"Подключить компьютер"));assertTrue(text(scanner.findViewById(android.R.id.content),"Назад"));});Thread.sleep(600);Bitmap shot=i.getUiAutomation().takeScreenshot();assertNotNull(shot);try(FileOutputStream out=new FileOutputStream(new File(scanner.getExternalFilesDir(null),"preview-qr-scanner.png"))){shot.compress(Bitmap.CompressFormat.PNG,100,out);}shot.recycle();}finally{i.runOnMainSync(scanner::finish);}
    }
}
