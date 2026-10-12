package com.diberlog.jarvis.mobile;

import android.content.Context;
import android.net.*;
import javax.net.ssl.*;
import java.security.*;
import java.security.cert.*;
import java.net.*;
import java.io.*;
import java.nio.charset.StandardCharsets;
import org.json.*;

final class SyncClient {
    static boolean privateIp(String host){String[] p=host.split("\\.");if(p.length!=4)return false;int[] n=new int[4];try{for(int i=0;i<4;i++){if(!p[i].matches("[0-9]{1,3}"))return false;n[i]=Integer.parseInt(p[i]);if(n[i]>255||!Integer.toString(n[i]).equals(p[i]))return false;}}catch(Exception e){return false;}return n[0]==10||(n[0]==172&&n[1]>=16&&n[1]<=31)||(n[0]==192&&n[1]==168);}
    static String fingerprint(byte[] bytes)throws Exception{StringBuilder s=new StringBuilder();for(byte b:MessageDigest.getInstance("SHA-256").digest(bytes))s.append(String.format(java.util.Locale.ROOT,"%02x",b&255));return s.toString();}
    static JSONObject parseQr(String raw)throws Exception{Uri u=Uri.parse(raw);if(!"jarvis-pair".equals(u.getScheme())||!"v1".equals(u.getHost())||raw.length()>1500)throw new IOException("Это не QR-код JARVIS");String host=u.getQueryParameter("host"),pin=u.getQueryParameter("fingerprint"),secret=u.getQueryParameter("secret");int port=Integer.parseInt(u.getQueryParameter("port"));if(host==null||!privateIp(host)||port<1024||port>65535||pin==null||!pin.matches("[a-f0-9]{64}")||secret==null||!secret.matches("[a-f0-9]{64}"))throw new IOException("Некорректный QR-код локальной сети");return new JSONObject().put("host",host).put("port",port).put("fingerprint",pin).put("secret",secret);}
    static void wifi(Context c)throws IOException{ConnectivityManager m=c.getSystemService(ConnectivityManager.class);NetworkCapabilities n=m.getNetworkCapabilities(m.getActiveNetwork());if(n==null||!n.hasTransport(NetworkCapabilities.TRANSPORT_WIFI))throw new IOException("Подключите телефон к той же Wi-Fi сети, что и компьютер");}
    static JSONObject request(Context c,JSONObject pairing,String path,String token,JSONObject data)throws Exception{
        wifi(c);String host=pairing.getString("host"),pin=pairing.getString("fingerprint");int port=pairing.getInt("port");if(!privateIp(host)||!pin.matches("[a-f0-9]{64}"))throw new IOException("Повреждены настройки подключения");
        X509TrustManager trust=new X509TrustManager(){public X509Certificate[] getAcceptedIssuers(){return new X509Certificate[0];}public void checkClientTrusted(X509Certificate[] chain,String auth)throws CertificateException{throw new CertificateException("Client TLS not supported");}public void checkServerTrusted(X509Certificate[] chain,String auth)throws CertificateException{try{if(chain==null||chain.length==0||!MessageDigest.isEqual(pin.getBytes(StandardCharsets.US_ASCII),fingerprint(chain[0].getEncoded()).getBytes(StandardCharsets.US_ASCII)))throw new CertificateException("Сертификат компьютера изменился. Подключитесь заново по QR-коду");chain[0].checkValidity();}catch(CertificateException e){throw e;}catch(Exception e){throw new CertificateException("Не удалось проверить компьютер");}}};
        SSLContext tls=SSLContext.getInstance("TLS");tls.init(null,new TrustManager[]{trust},new SecureRandom());
        HttpsURLConnection connection=(HttpsURLConnection)new URL("https://"+host+":"+port+path).openConnection();
        connection.setSSLSocketFactory(tls.getSocketFactory());
        // Private numeric endpoint from the QR + exact certificate pin. Never global trust bypass.
        connection.setHostnameVerifier((name,session)->{try{return host.equals(name)&&MessageDigest.isEqual(pin.getBytes(StandardCharsets.US_ASCII),fingerprint(session.getPeerCertificates()[0].getEncoded()).getBytes(StandardCharsets.US_ASCII));}catch(Exception e){return false;}});
        connection.setInstanceFollowRedirects(false);connection.setConnectTimeout(10000);connection.setReadTimeout(20000);connection.setRequestMethod("POST");connection.setDoOutput(true);connection.setRequestProperty("Content-Type","application/json; charset=utf-8");connection.setRequestProperty("Authorization","Bearer "+token);
        byte[] body=data.toString().getBytes(StandardCharsets.UTF_8);if(body.length>16*1024*1024)throw new IOException("Данные синхронизации больше 16 МБ. Уменьшите вложения");connection.setFixedLengthStreamingMode(body.length);
        try{try(OutputStream out=connection.getOutputStream()){out.write(body);}int status=connection.getResponseCode();InputStream input=status>=400?connection.getErrorStream():connection.getInputStream();if(input==null)throw new IOException("Компьютер не ответил");ByteArrayOutputStream bytes=new ByteArrayOutputStream();try(InputStream in=input){byte[] b=new byte[8192];int n;while((n=in.read(b))!=-1){if(bytes.size()+n>16*1024*1024)throw new IOException("Ответ больше 16 МБ");bytes.write(b,0,n);}}
            JSONObject result=new JSONObject(bytes.toString("UTF-8"));if(status>=400)throw new IOException(result.optString("error","Ошибка компьютера: HTTP "+status));result.put("httpStatus",status);return result;
        }finally{connection.disconnect();}
    }
    static void begin(Context c,String qr,String name)throws Exception{JSONObject p=parseQr(qr);JSONObject response=request(c,p,"/pair",p.getString("secret"),new JSONObject().put("name",name));p.put("pendingId",response.getString("id")).put("code",response.getString("code"));PrivateStorage.pairing(c,p);}
    static boolean finish(Context c)throws Exception{JSONObject p=PrivateStorage.pairing(c);if(p==null)throw new IOException("Сначала отсканируйте QR-код");JSONObject r=request(c,p,"/pair/status",p.getString("secret"),new JSONObject().put("id",p.getString("pendingId")));if(r.optInt("httpStatus")==202)return false;p.put("token",r.getString("token")).put("deviceId",r.getString("deviceId"));p.remove("secret");p.remove("pendingId");p.remove("code");PrivateStorage.pairing(c,p);MobileStore.prefs(c).edit().remove("syncBase").remove("syncBaseChats").remove("syncBaseVault").apply();return true;}
    static synchronized void sync(Context c)throws Exception{
        JSONObject p=PrivateStorage.pairing(c);if(p==null||!p.has("token"))throw new IOException("Сначала подтвердите сопряжение на компьютере");android.content.SharedPreferences prefs=MobileStore.prefs(c);
        JSONObject outbound=MobileStore.center(c);JSONObject base=new JSONObject(prefs.getString("syncBase",MobileStore.empty().toString()));JSONArray chats=MobileStore.chats(c),baseChats=new JSONArray(prefs.getString("syncBaseChats","[]"));JSONObject vault=PrivateStorage.vault(c);String baseVault=prefs.getString("syncBaseVault","null");boolean shareChats=prefs.getBoolean("syncChats",false),shareVault=prefs.getBoolean("syncVault",false);
        JSONObject payload=new JSONObject().put("version",1).put("base",base).put("center",outbound).put("syncChats",shareChats).put("chats",chats).put("baseChats",baseChats).put("syncVault",shareVault).put("vault",vault==null?JSONObject.NULL:vault).put("baseVault",baseVault.equals("null")?JSONObject.NULL:new JSONObject(baseVault));
        // Vault is omitted completely unless explicitly allowed on this phone.
        if(!shareVault){payload.put("vault",JSONObject.NULL).put("baseVault",JSONObject.NULL);}
        if(!shareChats){payload.put("chats",new JSONArray()).put("baseChats",new JSONArray());}
        JSONObject result=request(c,p,"/sync",p.getString("token"),payload);if(result.optInt("version")!=1)throw new IOException("Несовместимая версия синхронизации");JSONObject incoming=result.getJSONObject("center");MobileStore.validate(incoming);
        synchronized(MobileStore.class){JSONObject current=MobileStore.center(c);JSONObject merged=MobileStore.merge(outbound,current,incoming);if(!MobileStore.same(current,merged)){PrivateStorage.backup(c,"center",current.toString());MobileStore.save(c,merged);}if(shareChats){JSONArray remote=result.getJSONArray("chats");MobileStore.records(remote);JSONArray mergedChats=MobileStore.mergeRecords(chats,MobileStore.chats(c),remote,true);MobileStore.saveChats(c,mergedChats);}}
        boolean vaultConflict=false;
        if(shareVault&&result.optBoolean("vaultEnabled")){JSONObject remote=result.optJSONObject("vault");if(remote!=null){VaultCrypto.validate(remote);synchronized(PrivateStorage.class){JSONObject current=PrivateStorage.vault(c);if(result.optBoolean("vaultConflict")||!MobileStore.same(current,vault)){vaultConflict=true;prefs.edit().putString("vaultConflict",remote.toString()).commit();PrivateStorage.backup(c,"vault-conflict-encrypted",remote.toString());}else{PrivateStorage.vault(c,remote);prefs.edit().putString("syncBaseVault",remote.toString()).remove("vaultConflict").commit();}}}}
        prefs.edit().putString("syncBase",incoming.toString()).putString("syncBaseChats",shareChats?result.getJSONArray("chats").toString():prefs.getString("syncBaseChats","[]")).putString("syncConflicts",result.optJSONArray("conflicts")==null?"[]":result.getJSONArray("conflicts").toString()).putString("lastSync",MobileStore.now()).putString("lastSyncError","").commit();
        if(vaultConflict)throw new IOException("Обычные данные синхронизированы. Сейф имеет конфликт: разблокируйте его и объедините версии на телефоне");
        if(shareVault&&!result.optBoolean("vaultEnabled"))throw new IOException("Обычные данные синхронизированы. Разрешите обмен сейфом также на компьютере");
    }
}
