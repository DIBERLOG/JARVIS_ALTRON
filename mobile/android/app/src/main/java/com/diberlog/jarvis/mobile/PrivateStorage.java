package com.diberlog.jarvis.mobile;

import android.content.Context;
import android.util.AtomicFile;
import org.json.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import javax.crypto.*;
import javax.crypto.spec.*;
import android.security.keystore.*;
import java.security.KeyStore;

final class PrivateStorage {
    static synchronized void vaultExpected(Context c,JSONObject expected,JSONObject value)throws Exception{SyncJournal.saveVaultExpected(c,expected,value);}
    static JSONObject vaultLegacy(Context c)throws Exception{File f=new File(c.getFilesDir(),"password-vault-v1.json");return f.exists()?new JSONObject(read(f)):null;}
    static synchronized JSONObject vault(Context c)throws Exception{return SyncJournal.snapshot(c).optJSONObject("sync_vault");}
    static synchronized void vault(Context c,JSONObject value)throws Exception{VaultCrypto.validate(value);JSONObject old=vault(c);if(old!=null&&!MobileStore.same(old,value))backup(c,"vault",old.toString());SyncJournal.save(c,"sync_vault",value);}
    static void backup(Context c,String prefix,String text)throws Exception{File d=new File(c.getFilesDir(),"backups");if(!d.exists()&&!d.mkdirs())throw new IOException("Не удалось создать резервную копию");write(new File(d,prefix+"-"+System.currentTimeMillis()+"-"+MobileStore.id()+".json"),text);}
    static String read(File file)throws Exception{try(InputStream in=new FileInputStream(file);ByteArrayOutputStream out=new ByteArrayOutputStream()){byte[] b=new byte[8192];int n;while((n=in.read(b))!=-1){if(out.size()+n>16*1024*1024)throw new IOException("Файл больше 16 МБ");out.write(b,0,n);}return out.toString("UTF-8");}}
    static void write(File file,String value)throws Exception{AtomicFile a=new AtomicFile(file);FileOutputStream out=a.startWrite();try{out.write(value.getBytes(StandardCharsets.UTF_8));a.finishWrite(out);}catch(Exception e){a.failWrite(out);throw e;}}
    static SecretKey deviceKey()throws Exception{KeyStore s=KeyStore.getInstance("AndroidKeyStore");s.load(null);String alias="jarvis-pairing-v1";if(s.containsAlias(alias))return (SecretKey)s.getKey(alias,null);KeyGenerator g=KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES,"AndroidKeyStore");g.init(new KeyGenParameterSpec.Builder(alias,KeyProperties.PURPOSE_ENCRYPT|KeyProperties.PURPOSE_DECRYPT).setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setKeySize(256).build());return g.generateKey();}
    static synchronized JSONObject pairing(Context c)throws Exception{String raw=MobileStore.prefs(c).getString("pairing","");if(raw.isEmpty())return null;byte[] b=VaultCrypto.open(deviceKey(),new JSONObject(raw));try{return new JSONObject(new String(b,StandardCharsets.UTF_8));}finally{java.util.Arrays.fill(b,(byte)0);}}
    static synchronized void pairing(Context c,JSONObject value)throws Exception{String raw=value==null?"":VaultCrypto.seal(deviceKey(),value.toString().getBytes(StandardCharsets.UTF_8)).toString();if(!MobileStore.prefs(c).edit().putString("pairing",raw).commit())throw new IOException("Не удалось сохранить подключение");}
}
