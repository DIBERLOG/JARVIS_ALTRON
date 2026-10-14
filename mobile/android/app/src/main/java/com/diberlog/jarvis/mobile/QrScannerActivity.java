package com.diberlog.jarvis.mobile;

import android.graphics.*;
import android.view.*;
import android.widget.*;
import com.journeyapps.barcodescanner.*;

/** Foreground-only scanner; decoding, permission and result lifecycle stay with ZXing. */
public final class QrScannerActivity extends CaptureActivity {
    int dp(int value){return Math.round(value*getResources().getDisplayMetrics().density);}
    TextView text(String value,int size){TextView view=new TextView(this);view.setText(value);view.setTextColor(Color.WHITE);view.setTextSize(size);view.setGravity(Gravity.CENTER);view.setPadding(dp(20),dp(8),dp(20),dp(8));return view;}
    Button control(String title){Button button=new Button(this);button.setText(title);button.setAllCaps(false);button.setTextSize(15);button.setTextColor(Color.rgb(160,228,206));android.graphics.drawable.GradientDrawable background=new android.graphics.drawable.GradientDrawable();background.setColor(0xdd182327);background.setCornerRadius(dp(20));button.setBackground(background);button.setPadding(dp(16),dp(8),dp(16),dp(8));button.setMinHeight(dp(48));return button;}
    @Override protected DecoratedBarcodeView initializeContent(){
        FrameLayout root=new FrameLayout(this);root.setBackgroundColor(Color.rgb(16,19,21));
        DecoratedBarcodeView scanner=new DecoratedBarcodeView(this);scanner.getViewFinder().setVisibility(View.GONE);if(scanner.getStatusView()!=null)scanner.getStatusView().setVisibility(View.GONE);
        scanner.getBarcodeView().setFramingRectSize(new Size(dp(260),dp(260)));root.addView(scanner,new FrameLayout.LayoutParams(-1,-1));
        Guide guide=new Guide(scanner);guide.setTag("qr-capture-guide");guide.setContentDescription("Рамка захвата QR-кода JARVIS");root.addView(guide,new FrameLayout.LayoutParams(-1,-1));
        LinearLayout top=new LinearLayout(this);top.setGravity(Gravity.CENTER_VERTICAL);Button close=control("‹ Назад");close.setContentDescription("Закрыть сканер");close.setOnClickListener(v->finish());top.addView(close);TextView title=text("Подключить компьютер",18);top.addView(title,new LinearLayout.LayoutParams(0,-2,1));FrameLayout.LayoutParams topPosition=new FrameLayout.LayoutParams(-1,-2,Gravity.TOP);topPosition.setMargins(dp(16),dp(16),dp(16),0);root.addView(top,topPosition);
        LinearLayout bottom=new LinearLayout(this);bottom.setOrientation(LinearLayout.VERTICAL);bottom.setGravity(Gravity.CENTER);bottom.addView(text("Наведите камеру на QR-код\nв настройках JARVIS на компьютере",17));bottom.addView(text("Код распознаётся автоматически",13));
        if(getPackageManager().hasSystemFeature(android.content.pm.PackageManager.FEATURE_CAMERA_FLASH)){Button torch=control("Включить фонарик");boolean[] lit={false};torch.setOnClickListener(v->{lit[0]=!lit[0];if(lit[0])scanner.setTorchOn();else scanner.setTorchOff();torch.setText(lit[0]?"Выключить фонарик":"Включить фонарик");});bottom.addView(torch);}
        FrameLayout.LayoutParams bottomPosition=new FrameLayout.LayoutParams(-1,-2,Gravity.BOTTOM);bottomPosition.setMargins(dp(16),0,dp(16),dp(24));root.addView(bottom,bottomPosition);
        root.setOnApplyWindowInsetsListener((v,insets)->{v.setPadding(insets.getSystemWindowInsetLeft(),insets.getSystemWindowInsetTop(),insets.getSystemWindowInsetRight(),insets.getSystemWindowInsetBottom());return insets;});
        setContentView(root);getWindow().setStatusBarColor(Color.rgb(16,19,21));getWindow().setNavigationBarColor(Color.rgb(16,19,21));return scanner;
    }
    final class Guide extends View {
        final DecoratedBarcodeView scanner;final Paint paint=new Paint(Paint.ANTI_ALIAS_FLAG);final boolean animate;
        Guide(DecoratedBarcodeView scanner){super(QrScannerActivity.this);this.scanner=scanner;animate=android.provider.Settings.Global.getFloat(getContentResolver(),android.provider.Settings.Global.ANIMATOR_DURATION_SCALE,1)>0;}
        @Override protected void onDraw(Canvas canvas){super.onDraw(canvas);Rect area=scanner.getBarcodeView().getFramingRect();float size=Math.min(dp(260),Math.min(getWidth(),getHeight())*.7f);RectF frame=area==null?new RectF((getWidth()-size)/2,(getHeight()-size)/2,(getWidth()+size)/2,(getHeight()+size)/2):new RectF(area);
            Path mask=new Path();mask.setFillType(Path.FillType.EVEN_ODD);mask.addRect(0,0,getWidth(),getHeight(),Path.Direction.CW);mask.addRoundRect(frame,dp(22),dp(22),Path.Direction.CW);paint.setStyle(Paint.Style.FILL);paint.setColor(0x99080e10);canvas.drawPath(mask,paint);
            paint.setStyle(Paint.Style.STROKE);paint.setColor(0xffa0e4ce);paint.setStrokeWidth(dp(2));canvas.drawRoundRect(frame,dp(22),dp(22),paint);paint.setStrokeWidth(dp(5));paint.setStrokeCap(Paint.Cap.ROUND);float diameter=dp(44);canvas.drawArc(frame.left,frame.top,frame.left+diameter,frame.top+diameter,180,90,false,paint);canvas.drawArc(frame.right-diameter,frame.top,frame.right,frame.top+diameter,270,90,false,paint);canvas.drawArc(frame.right-diameter,frame.bottom-diameter,frame.right,frame.bottom,0,90,false,paint);canvas.drawArc(frame.left,frame.bottom-diameter,frame.left+diameter,frame.bottom,90,90,false,paint);
            paint.setStyle(Paint.Style.FILL);paint.setColor(0xcc8ee8d0);float phase=animate?(android.os.SystemClock.uptimeMillis()%2400)/2400f:.5f;float y=frame.top+dp(18)+phase*(frame.height()-dp(36));canvas.drawRoundRect(frame.left+dp(18),y,frame.right-dp(18),y+dp(2),dp(1),dp(1),paint);
            if(animate&&getWindowVisibility()==View.VISIBLE)postInvalidateDelayed(32);
        }
    }
}
