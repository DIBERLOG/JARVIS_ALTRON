package com.diberlog.jarvis.mobile;

import android.app.Instrumentation;
import android.content.Intent;
import android.graphics.Bitmap;
import android.view.View;
import android.view.WindowManager;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import org.json.JSONArray;
import org.json.JSONObject;
import org.junit.Test;
import org.junit.runner.RunWith;
import java.io.File;
import java.io.FileOutputStream;
import static org.junit.Assert.*;

/** Run on a dedicated fictional-data emulator, never on a personal profile. */
@RunWith(AndroidJUnit4.class)
public class NavigationTests {
    final Instrumentation instrumentation=InstrumentationRegistry.getInstrumentation();
    MainActivity open(){return (MainActivity)instrumentation.startActivitySync(new Intent(instrumentation.getTargetContext(),MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));}
    @Test public void homeAndSystemBackWorkInEverySection(){MainActivity activity=open();try{instrumentation.runOnMainSync(()->{
        for(int section=1;section<MainActivity.SECTIONS.length;section++){
            activity.show(section);
            assertEquals(section,activity.tab);
            View home=activity.shell.findViewWithTag("nav-0");assertNotNull(home);assertTrue(home.performClick());assertEquals(0,activity.tab);
            activity.show(section);activity.getOnBackPressedDispatcher().onBackPressed();assertEquals(0,activity.tab);
        }
        activity.show(11);assertNotEquals(0,activity.getWindow().getAttributes().flags&WindowManager.LayoutParams.FLAG_SECURE);
        activity.shell.findViewWithTag("nav-0").performClick();assertEquals(0,activity.getWindow().getAttributes().flags&WindowManager.LayoutParams.FLAG_SECURE);
    });}finally{instrumentation.runOnMainSync(activity::finish);}}
    @Test public void composerStaysOutsideScrollAndDraftSurvivesNavigation(){MainActivity activity=open();try{instrumentation.runOnMainSync(()->{try{
        activity.show(1);int chats=MobileStore.chats(activity).length();
        assertNotNull(activity.input);assertFalse(isInside(activity.input,activity.contentScroll));
        assertTrue(activity.input.getMinHeight()>=activity.dp(48));
        activity.input.setText("Вымышленный черновик");activity.shell.findViewWithTag("nav-0").performClick();activity.show(1);
        assertEquals("Вымышленный черновик",activity.input.getText().toString());assertEquals(chats,MobileStore.chats(activity).length());
        activity.newChat();assertEquals("",activity.input.getText().toString());
        assertEquals("Отправить сообщение",activity.send.getContentDescription().toString());
    }catch(Exception e){throw new AssertionError(e);}});}finally{instrumentation.runOnMainSync(activity::finish);}}
    boolean isInside(View child,View ancestor){for(android.view.ViewParent parent=child.getParent();parent!=null;parent=parent.getParent())if(parent==ancestor)return true;return false;}
    @Test public void syncGuideAndRemoteCommandsHaveSeparateScreens()throws Exception{
        MainActivity a=open();JSONObject old=PrivateStorage.pairing(a);boolean auto=a.prefs.getBoolean("autoSync",false);
        try{a.prefs.edit().putBoolean("autoSync",false).commit();PrivateStorage.pairing(a,null);
            instrumentation.runOnMainSync(()->{a.show(14);for(int n=0;n<3;n++)assertNotNull(a.page.findViewWithTag("sync-guide-"+n));assertEquals("Как работает синхронизация",MainActivity.SECTIONS[14]);a.show(15);assertNull(a.page.findViewWithTag("remote-command-fixture"));assertNull(a.remoteCommands);assertEquals("Команды компьютеру",MainActivity.SECTIONS[15]);});
            PrivateStorage.pairing(a,new JSONObject().put("host","192.168.1.42").put("port",47832).put("token","fictional-token"));
            JSONArray commands=new JSONArray().put(new JSONObject().put("id","fixture").put("title","Тестовая разрешённая команда"));
            instrumentation.runOnMainSync(()->{a.remoteCommands=commands;a.show(15);assertNotNull(a.page.findViewWithTag("remote-command-fixture"));});
            PolishTests.shot(instrumentation,a,"preview-commands-section.png");
            instrumentation.runOnMainSync(()->{a.show(14);assertNull(a.remoteCommands);});PolishTests.shot(instrumentation,a,"preview-sync-guide.png");
        }finally{PrivateStorage.pairing(a,old);a.prefs.edit().putBoolean("autoSync",auto).commit();instrumentation.runOnMainSync(a::finish);}
    }
    @Test public void keyboardLeavesMessageAndSendButtonVisible()throws Exception{MainActivity activity=open();try{
        for(int n=0;n<30;n++){java.util.concurrent.atomic.AtomicBoolean focused=new java.util.concurrent.atomic.AtomicBoolean();instrumentation.runOnMainSync(()->focused.set(activity.hasWindowFocus()));if(focused.get())break;Thread.sleep(100);}
        instrumentation.runOnMainSync(()->{activity.show(1);activity.input.requestFocus();((android.view.inputmethod.InputMethodManager)activity.getSystemService(android.content.Context.INPUT_METHOD_SERVICE)).showSoftInput(activity.input,android.view.inputmethod.InputMethodManager.SHOW_IMPLICIT);});
        instrumentation.waitForIdleSync();for(int n=0;n<30;n++){java.util.concurrent.atomic.AtomicBoolean visible=new java.util.concurrent.atomic.AtomicBoolean();instrumentation.runOnMainSync(()->{visible.set(android.os.Build.VERSION.SDK_INT<30||activity.shell.getRootWindowInsets()!=null&&activity.shell.getRootWindowInsets().isVisible(android.view.WindowInsets.Type.ime()));if(!visible.get())((android.view.inputmethod.InputMethodManager)activity.getSystemService(android.content.Context.INPUT_METHOD_SERVICE)).showSoftInput(activity.input,android.view.inputmethod.InputMethodManager.SHOW_IMPLICIT);});if(visible.get())break;Thread.sleep(150);}Thread.sleep(250);
        instrumentation.runOnMainSync(()->{android.graphics.Rect visible=new android.graphics.Rect();activity.shell.getWindowVisibleDisplayFrame(visible);int[] location=new int[2];activity.send.getLocationOnScreen(location);assertTrue("Send button must stay above keyboard",location[1]+activity.send.getHeight()<=visible.bottom);assertTrue(activity.input.isShown());if(android.os.Build.VERSION.SDK_INT>=30){assertTrue(activity.shell.getRootWindowInsets().isVisible(android.view.WindowInsets.Type.ime()));assertEquals(View.GONE,activity.bottomNav.getVisibility());}});
        screenshot(activity,"ui-keyboard.png");
        instrumentation.runOnMainSync(()->activity.show(0));
    }finally{instrumentation.runOnMainSync(activity::finish);}}
    void screenshot(MainActivity activity,String name)throws Exception{instrumentation.waitForIdleSync();Thread.sleep(350);Bitmap shot=instrumentation.getUiAutomation().takeScreenshot();assertNotNull(shot);try(FileOutputStream out=new FileOutputStream(new File(activity.getFilesDir(),name))){assertTrue(shot.compress(Bitmap.CompressFormat.PNG,100,out));}shot.recycle();}
    @Test public void captureFictionalUiPreviews()throws Exception{MainActivity activity=open();try{
        instrumentation.runOnMainSync(()->activity.show(0));screenshot(activity,"ui-home.png");
        instrumentation.runOnMainSync(()->{try{activity.show(1);activity.renderMessages(new JSONArray().put(new JSONObject().put("role","user").put("content","Помоги спланировать день без перегрузки.")).put(new JSONObject().put("role","assistant").put("content","Начнём с трёх приоритетов.\n\n1. Выберите одно главное дело.\n2. Оставьте время на отдых.\n3. Перенесите всё необязательное на завтра.\n\nКакое дело сейчас самое важное?")));}catch(Exception e){throw new AssertionError(e);}});screenshot(activity,"ui-chat.png");
        instrumentation.runOnMainSync(()->activity.show(10));screenshot(activity,"ui-training.png");
    }finally{instrumentation.runOnMainSync(activity::finish);}}
}
