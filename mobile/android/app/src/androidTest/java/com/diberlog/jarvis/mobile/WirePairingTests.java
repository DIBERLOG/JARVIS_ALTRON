package com.diberlog.jarvis.mobile;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import org.json.JSONObject;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;

/** Explicit opt-in: a Rust TLS fixture on localhost, never the user's hub. */
@RunWith(AndroidJUnit4.class)
public class WirePairingTests {
    @Test public void androidPostPairsWithRustAndRejectsWrongSecretAndCertificate()throws Exception{
        String encoded=InstrumentationRegistry.getArguments().getString("wireFixtureBase64");org.junit.Assume.assumeNotNull(encoded);
        String fixture=new String(android.util.Base64.decode(encoded,android.util.Base64.NO_WRAP),java.nio.charset.StandardCharsets.UTF_8);
        assertEquals("10.0.2.2",new JSONObject(fixture).getString("host"));
        android.content.Context context=InstrumentationRegistry.getInstrumentation().getTargetContext();JSONObject body=new JSONObject().put("version",2).put("name","Тестовый телефон");
        for(boolean route:new boolean[]{false,true}){JSONObject pairing=new JSONObject(fixture);
        JSONObject result=SyncClient.request(context,pairing,"/pair",pairing.getString("secret"),body,route);assertEquals(202,result.getInt("httpStatus"));assertFalse(result.getString("id").isEmpty());
        try{SyncClient.request(context,pairing,"/pair","fictional-wrong-secret",body,route);fail("Wrong secret accepted");}catch(java.io.IOException expected){assertTrue(expected.getMessage().contains("QR-код"));}
        String pin=pairing.getString("fingerprint");pairing.put("fingerprint",(pin.startsWith("0")?"1":"0")+pin.substring(1));
        try{SyncClient.request(context,pairing,"/pair",pairing.getString("secret"),body,route);fail("Wrong certificate accepted");}catch(javax.net.ssl.SSLException expected){}
        }
    }
}
