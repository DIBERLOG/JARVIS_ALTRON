package com.diberlog.jarvis.mobile;

import android.util.Base64;
import org.json.*;
import javax.crypto.*;
import javax.crypto.spec.*;
import java.security.SecureRandom;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;

/** Desktop v1 compatible: PBKDF2-SHA256/600000 + AES-256-GCM, fresh 96-bit nonce. */
final class VaultCrypto {
    static final SecureRandom RANDOM=new SecureRandom();
    static byte[] random(int n){byte[] b=new byte[n];RANDOM.nextBytes(b);return b;}
    static String b64(byte[] b){return Base64.encodeToString(b,Base64.NO_WRAP);}
    static byte[] bytes(String s){return Base64.decode(s,Base64.NO_WRAP);}
    static SecretKey password(char[] password,String salt)throws Exception{
        PBEKeySpec spec=new PBEKeySpec(password,bytes(salt),600000,256);try{byte[] b=SecretKeyFactory.getInstance("PBKDF2WithHmacSHA256").generateSecret(spec).getEncoded();try{return new SecretKeySpec(b,"AES");}finally{Arrays.fill(b,(byte)0);}}finally{spec.clearPassword();}
    }
    static JSONObject seal(SecretKey key,byte[] plain)throws Exception {Cipher c=Cipher.getInstance("AES/GCM/NoPadding");if(key.getEncoded()==null)c.init(Cipher.ENCRYPT_MODE,key);else c.init(Cipher.ENCRYPT_MODE,key,new GCMParameterSpec(128,random(12)));return new JSONObject().put("iv",b64(c.getIV())).put("data",b64(c.doFinal(plain)));}
    static byte[] open(SecretKey key,JSONObject sealed)throws Exception{byte[] iv=bytes(sealed.getString("iv"));if(iv.length!=12)throw new IllegalArgumentException("Повреждён сейф");Cipher c=Cipher.getInstance("AES/GCM/NoPadding");c.init(Cipher.DECRYPT_MODE,key,new GCMParameterSpec(128,iv));return c.doFinal(bytes(sealed.getString("data")));}
    static JSONObject create(char[] pass)throws Exception{if(pass.length<6||pass.length>1024)throw new IllegalArgumentException("Мастер-пароль: от 6 до 1024 символов");byte[] raw=random(32),recovery=random(32);try{String salt=b64(random(16));SecretKey data=new SecretKeySpec(raw,"AES");return new JSONObject().put("version",1).put("salt",salt).put("passwordKey",seal(password(pass,salt),raw)).put("recoveryKey",seal(new SecretKeySpec(recovery,"AES"),raw)).put("cards",seal(data,"[]".getBytes(StandardCharsets.UTF_8)));}finally{Arrays.fill(raw,(byte)0);Arrays.fill(recovery,(byte)0);}}
    static SecretKey unlock(JSONObject f,char[] pass)throws Exception{validate(f);byte[] raw=open(password(pass,f.getString("salt")),f.getJSONObject("passwordKey"));try{if(raw.length!=32)throw new IllegalArgumentException("Повреждён ключ сейфа");return new SecretKeySpec(raw,"AES");}finally{Arrays.fill(raw,(byte)0);}}
    static JSONArray cards(JSONObject f,SecretKey k)throws Exception{byte[] raw=open(k,f.getJSONObject("cards"));try{JSONArray cards=new JSONArray(new String(raw,StandardCharsets.UTF_8));MobileStore.records(cards);return cards;}finally{Arrays.fill(raw,(byte)0);}}
    static JSONObject update(JSONObject f,SecretKey k,JSONArray cards)throws Exception{byte[] raw=cards.toString().getBytes(StandardCharsets.UTF_8);try{if(raw.length>4_000_000)throw new IllegalArgumentException("Мобильный сейф больше 4 МБ");return new JSONObject(f.toString()).put("cards",seal(k,raw));}finally{Arrays.fill(raw,(byte)0);}}
    static void validate(JSONObject f)throws Exception{if(f.getInt("version")!=1||bytes(f.getString("salt")).length!=16)throw new IllegalArgumentException("Неизвестный формат сейфа");for(String k:new String[]{"passwordKey","recoveryKey","cards"}){JSONObject s=f.getJSONObject(k);int size=bytes(s.getString("data")).length;if(bytes(s.getString("iv")).length!=12||size<16||(!k.equals("cards")&&size!=48))throw new IllegalArgumentException("Повреждён сейф");}}
}
