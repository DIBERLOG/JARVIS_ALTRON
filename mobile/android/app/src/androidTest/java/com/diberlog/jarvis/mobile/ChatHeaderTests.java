package com.diberlog.jarvis.mobile;

import android.content.Intent;
import android.view.View;
import android.widget.*;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

@RunWith(AndroidJUnit4.class)
public class ChatHeaderTests {
    @Test public void providerAndModelStayReadableAtNarrowWidthsAndLargeText()throws Exception {
        android.app.Instrumentation i=InstrumentationRegistry.getInstrumentation();
        MainActivity a=(MainActivity)i.startActivitySync(new Intent(i.getTargetContext(),MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
        String provider=a.prefs.getString("provider","OpenAI"),model=a.prefs.getString("model","gpt-4.1-mini"),theme=a.prefs.getString("appearance","dark");
        try {
            i.runOnMainSync(()->{a.prefs.edit().putString("provider","DeepSeek").putString("model","deepseek-flash").apply();MobileTheme.choose(a,"dark");a.show(1);});
            i.waitForIdleSync();
            i.runOnMainSync(()->{
                LinearLayout heading=a.shell.findViewWithTag("chat-header");TextView subtitle=a.shell.findViewWithTag("chat-provider");TextView title=a.shell.findViewWithTag("chat-title");
                assertEquals("DeepSeek · deepseek-flash",subtitle.getText().toString());
                for(int width:new int[]{320,360})for(float scale:new float[]{1,1.5f}){
                    subtitle.setTextSize(12*scale);title.setTextSize(17*scale);
                    heading.measure(View.MeasureSpec.makeMeasureSpec(a.dp(width),View.MeasureSpec.EXACTLY),View.MeasureSpec.makeMeasureSpec(0,View.MeasureSpec.UNSPECIFIED));heading.layout(0,0,a.dp(width),heading.getMeasuredHeight());
                    assertNotNull(subtitle.getLayout());assertEquals(a.dp(width),subtitle.getWidth());
                    assertTrue("Provider name must fit visibly",subtitle.getLayout().getLineEnd(0)>=8);
                    assertTrue("Text bottom must not be clipped",subtitle.getLayout().getLineBottom(subtitle.getLayout().getLineCount()-1)+subtitle.getTotalPaddingBottom()<=subtitle.getHeight());
                    assertTrue("Model row must stay below actions",subtitle.getTop()>=heading.getChildAt(0).getBottom());
                }
                subtitle.setTextSize(12);title.setTextSize(17);heading.requestLayout();
            });
            PolishTests.shot(i,a,"preview-chat-header-deepseek.png");
        } finally {a.prefs.edit().putString("provider",provider).putString("model",model).putString("appearance",theme).apply();i.runOnMainSync(a::finish);}
    }
}
