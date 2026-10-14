package com.diberlog.jarvis.mobile;

import android.content.res.Configuration;
import android.graphics.Color;
import android.view.*;

final class MobileTheme {
    static boolean light(MainActivity a){String choice=MobileStore.prefs(a).getString("appearance","dark");return choice.equals("light")||choice.equals("system")&&(a.getResources().getConfiguration().uiMode&Configuration.UI_MODE_NIGHT_MASK)!=Configuration.UI_MODE_NIGHT_YES;}
    static void apply(MainActivity a){boolean light=light(a);a.setTheme(light?R.style.Theme_Jarvis_Light:R.style.Theme_Jarvis);
        MainActivity.BG=Color.parseColor(light?"#F1F5F4":"#101315");MainActivity.PANEL=Color.parseColor(light?"#FFFFFF":"#1B2023");MainActivity.WHITE=Color.parseColor(light?"#172B30":"#F1F4F5");MainActivity.MUTED=Color.parseColor(light?"#455E65":"#B4C2C7");MainActivity.BORDER=Color.parseColor(light?"#B7CECC":"#313A3E");MainActivity.TEAL=Color.parseColor(light?"#176D60":"#A0E4CE");
    }
    static void bars(MainActivity a){View decor=a.getWindow().getDecorView();a.getWindow().setStatusBarColor(MainActivity.BG);a.getWindow().setNavigationBarColor(MainActivity.BG);if(android.os.Build.VERSION.SDK_INT>=30){WindowInsetsController control=a.getWindow().getInsetsController();if(control!=null){int mask=WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS|WindowInsetsController.APPEARANCE_LIGHT_NAVIGATION_BARS;control.setSystemBarsAppearance(light(a)?mask:0,mask);}}int bits=View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR|View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;decor.setSystemUiVisibility(light(a)?decor.getSystemUiVisibility()|bits:decor.getSystemUiVisibility()&~bits);}
    static int[] colors(){return new int[]{MainActivity.BG,MainActivity.PANEL,MainActivity.WHITE,MainActivity.MUTED,MainActivity.BORDER,MainActivity.TEAL};}
    static int color(int value,int[] old){int[] next=colors();for(int n=0;n<old.length;n++)if(value==old[n])return next[n];return value;}
    static void recolor(android.view.View v,int[] old){if(v instanceof android.widget.TextView){android.widget.TextView text=(android.widget.TextView)v;text.setTextColor(color(text.getCurrentTextColor(),old));text.setHintTextColor(color(text.getCurrentHintTextColor(),old));}if(v instanceof android.widget.ImageView){android.widget.ImageView image=(android.widget.ImageView)v;if(image.getImageTintList()!=null)image.setImageTintList(android.content.res.ColorStateList.valueOf(color(image.getImageTintList().getDefaultColor(),old)));}drawable(v.getBackground(),old);if(v instanceof android.view.ViewGroup){android.view.ViewGroup group=(android.view.ViewGroup)v;for(int n=0;n<group.getChildCount();n++)recolor(group.getChildAt(n),old);}v.invalidate();}
    static void drawable(android.graphics.drawable.Drawable d,int[] old){if(d instanceof android.graphics.drawable.GradientDrawable){android.graphics.drawable.GradientDrawable gradient=(android.graphics.drawable.GradientDrawable)d;if(gradient.getColor()!=null)gradient.setColor(color(gradient.getColor().getDefaultColor(),old));}else if(d instanceof android.graphics.drawable.LayerDrawable){android.graphics.drawable.LayerDrawable layers=(android.graphics.drawable.LayerDrawable)d;for(int n=0;n<layers.getNumberOfLayers();n++)drawable(layers.getDrawable(n),old);}}
    static void choose(MainActivity a,String value){if(!value.equals("dark")&&!value.equals("light")&&!value.equals("system"))return;a.prefs.edit().putString("appearance",value).apply();apply(a);bars(a);a.show(a.tab);}
}
