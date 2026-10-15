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
    static JSONObject parseQr(String raw)throws Exception{Uri u=Uri.parse(raw);if(!"jarvis-pair".equals(u.getScheme())||!"v2".equals(u.getHost())||raw.length()>1500)throw new IOException("Это не QR-код JARVIS");String host=u.getQueryParameter("host"),pin=u.getQueryParameter("fingerprint"),secret=u.getQueryParameter("secret");int port=Integer.parseInt(u.getQueryParameter("port"));if(host==null||!privateIp(host)||port<1024||port>65535||pin==null||!pin.matches("[a-f0-9]{64}")||secret==null||!secret.matches("[a-f0-9]{64}"))throw new IOException("Некорректный QR-код локальной сети");return new JSONObject().put("host",host).put("port",port).put("fingerprint",pin).put("secret",secret);}
    static JSONObject request(Context c,JSONObject pairing,String path,String token,JSONObject data)throws Exception{
        try{return request(c,pairing,path,token,data,false);}catch(IOException error){if(!bindingDenied(error))throw error;try{return request(c,pairing,path,token,data,true);}catch(IOException fallback){if(bindingDenied(fallback))throw new IOException("Android запрещает соединение с компьютером из-за VPN. Разрешите локальную сеть в VPN или временно отключите VPN для проверки обмена.",fallback);throw fallback;}}
    }
    // Retry only an OS refusal to bind the socket, before any HTTP data can be sent.
    // The normal route still requires the exact PC certificate and never uses an HTTP proxy.
    static boolean bindingDenied(Throwable error){for(Throwable cause=error;cause!=null;cause=cause.getCause()){String message=cause.getMessage();if(message!=null&&message.contains("Binding socket to network")&&(message.contains("EPERM")||message.contains("EACCES")))return true;}return false;}
    static JSONObject request(Context c,JSONObject pairing,String path,String token,JSONObject data,boolean defaultRoute)throws Exception{
        android.net.Network localWifi=wifi(c);if(localWifi==null)throw new IOException("Подключите телефон к той же Wi-Fi сети, что и компьютер");String host=pairing.getString("host"),pin=pairing.getString("fingerprint");int port=pairing.getInt("port");if(!privateIp(host)||!pin.matches("[a-f0-9]{64}"))throw new IOException("Повреждены настройки подключения");
        X509TrustManager trust=new X509TrustManager(){public X509Certificate[] getAcceptedIssuers(){return new X509Certificate[0];}public void checkClientTrusted(X509Certificate[] chain,String auth)throws CertificateException{throw new CertificateException("Client TLS not supported");}public void checkServerTrusted(X509Certificate[] chain,String auth)throws CertificateException{try{if(chain==null||chain.length==0||!MessageDigest.isEqual(pin.getBytes(StandardCharsets.US_ASCII),fingerprint(chain[0].getEncoded()).getBytes(StandardCharsets.US_ASCII)))throw new CertificateException("Сертификат компьютера изменился. Подключитесь заново по QR-коду");chain[0].checkValidity();}catch(CertificateException e){throw e;}catch(Exception e){throw new CertificateException("Не удалось проверить компьютер");}}};
        SSLContext tls=SSLContext.getInstance("TLS");tls.init(null,new TrustManager[]{trust},new SecureRandom());
        URL address=new URL("https://"+host+":"+port+path);HttpsURLConnection connection=(HttpsURLConnection)(defaultRoute?address.openConnection(java.net.Proxy.NO_PROXY):localWifi.openConnection(address,java.net.Proxy.NO_PROXY));
        connection.setSSLSocketFactory(tls.getSocketFactory());
        // Private numeric endpoint from the QR + exact certificate pin. Never global trust bypass.
        connection.setHostnameVerifier((name,session)->{try{return host.equals(name)&&MessageDigest.isEqual(pin.getBytes(StandardCharsets.US_ASCII),fingerprint(session.getPeerCertificates()[0].getEncoded()).getBytes(StandardCharsets.US_ASCII));}catch(Exception e){return false;}});
        connection.setInstanceFollowRedirects(false);connection.setConnectTimeout(10000);connection.setReadTimeout(20000);connection.setRequestMethod("POST");connection.setDoOutput(true);connection.setRequestProperty("Content-Type","application/json; charset=utf-8");connection.setRequestProperty("Authorization","Bearer "+token);
        byte[] body=data.toString().getBytes(StandardCharsets.UTF_8);if(body.length>16*1024*1024)throw new IOException("Данные синхронизации больше 16 МБ. Уменьшите вложения");connection.setFixedLengthStreamingMode(body.length);
        try{try(OutputStream out=connection.getOutputStream()){out.write(body);}int status=connection.getResponseCode();InputStream input=status>=400?connection.getErrorStream():connection.getInputStream();if(input==null)throw new IOException("Компьютер не ответил");ByteArrayOutputStream bytes=new ByteArrayOutputStream();try(InputStream in=input){byte[] b=new byte[8192];int n;while((n=in.read(b))!=-1){if(bytes.size()+n>16*1024*1024)throw new IOException("Ответ больше 16 МБ");bytes.write(b,0,n);}}
            JSONObject result=new JSONObject(bytes.toString("UTF-8"));if(status>=400)throw new IOException(result.optString("error","Ошибка компьютера: HTTP "+status));result.put("httpStatus",status);return result;
        }catch(java.net.SocketTimeoutException|java.net.ConnectException error){if(!defaultRoute&&bindingDenied(error))throw error;throw new IOException("Компьютер не ответил по адресу "+host+":"+port+". На ПК включите обмен и заново отсканируйте QR-код. Устройства должны быть в одной Wi-Fi сети.",error);}catch(IOException error){if(defaultRoute&&bindingDenied(error))throw new IOException("Android запрещает соединение с компьютером из-за VPN. Разрешите локальную сеть в VPN. Если включена блокировка соединений без VPN, исключения приложений могут не работать. Для проверки временно отключите VPN и повторите обмен.",error);throw error;}finally{connection.disconnect();}
    }
    static void validateDirection(String mode)throws IOException{if(!mode.equals("both")&&!mode.equals("from_pc")&&!mode.equals("to_pc"))throw new IOException("Выберите направление синхронизации заново");}
    static String directionTitle(String mode){return mode.equals("from_pc")?"ПК → телефон":mode.equals("to_pc")?"Телефон → ПК":mode.equals("both")?"В обе стороны":"Не выбрано";}
    static JSONObject packet(Context c,String mode)throws Exception{validateDirection(mode);JSONObject out=SyncJournal.outbound(c);if(mode.equals("from_pc"))out.put("operations",new JSONArray()).put("recovering",false);return out;}
    static boolean supportsTasks(JSONObject keys){JSONArray features=keys.optJSONArray("features");if(features!=null)for(int n=0;n<features.length();n++)if(features.optString(n).equals("tasks"))return true;return false;}
    static boolean physicalWifi(android.net.NetworkCapabilities caps){return caps!=null&&caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_WIFI)&&!caps.hasTransport(android.net.NetworkCapabilities.TRANSPORT_VPN);}
    static android.net.Network wifi(Context c){android.net.ConnectivityManager manager=c.getSystemService(android.net.ConnectivityManager.class);for(android.net.Network network:manager.getAllNetworks()){android.net.NetworkCapabilities caps=manager.getNetworkCapabilities(network);if(physicalWifi(caps))return network;}return null;}
    static void begin(Context c,String qr,String name)throws Exception{JSONObject p=parseQr(qr);JSONObject response=request(c,p,"/pair",p.getString("secret"),new JSONObject().put("version",2).put("name",name));p.put("pendingId",response.getString("id")).put("code",response.getString("code"));PrivateStorage.pairing(c,p);}
    static boolean checkApproval(Context c)throws Exception{
        JSONObject p=PrivateStorage.pairing(c);if(p==null)throw new IOException("Сначала отсканируйте QR-код");
        if(p.has("token"))return true;JSONObject r=p.optJSONObject("approved");if(r==null){r=request(c,p,"/pair/status",p.getString("secret"),new JSONObject().put("id",p.getString("pendingId")));if(r.optInt("httpStatus")==202)return false;completedPairing(p,r);p.put("approved",r);PrivateStorage.pairing(c,p);}return true;
    }
    static boolean finish(Context c)throws Exception{
        if(!checkApproval(c))return false;
        JSONObject p=PrivateStorage.pairing(c);if(p.has("token"))return true;JSONObject r=p.getJSONObject("approved");
        JSONObject completed=completedPairing(p,r);
        SyncJournal.provision(c,r.getJSONObject("group"),r.getString("deviceId"));
        PrivateStorage.pairing(c,completed);if(!MobileStore.prefs(c).contains("autoSync"))MobileStore.prefs(c).edit().putBoolean("autoSync",true).apply();SyncWorker.configure(c);return true;
    }
    static JSONObject completedPairing(JSONObject pending,JSONObject approved)throws Exception{
        if(approved.optInt("version")!=2||approved.optString("token").isEmpty()||approved.optString("deviceId").isEmpty())throw new IOException("Компьютер не выдал полное подтверждение. Подключение пока не завершено.");
        JSONObject completed=new JSONObject(pending.toString()).put("token",approved.getString("token")).put("deviceId",approved.getString("deviceId"));for(String key:new String[]{"secret","pendingId","code","approved"})completed.remove(key);return completed;
    }
    static synchronized void sync(Context c)throws Exception{
        String mode=MobileStore.prefs(c).getString("syncDirection","both");validateDirection(mode);boolean pull=!mode.equals("to_pc");
        JSONObject p=PrivateStorage.pairing(c);if(p==null||!p.has("token"))throw new IOException("Сначала подтвердите сопряжение на компьютере");
        JSONObject keys=request(c,p,"/v2/keys",p.getString("token"),new JSONObject().put("version",2));
        SyncJournal.provision(c,keys.getJSONObject("group"),p.getString("deviceId"));SyncJournal.relay(c,keys.getString("relay_id"));
        android.content.SharedPreferences prefs=MobileStore.prefs(c);SyncJournal.sharing(c,prefs.getBoolean("syncChats",false),prefs.getBoolean("syncVault",false));
        for(int i=0;i<100;i++){
            JSONObject outgoing=packet(c,mode);JSONArray ops=outgoing.getJSONArray("operations");for(int n=0;n<ops.length();n++)if(ops.getJSONObject(n).getString("collection").equals("tasks")&&!supportsTasks(keys))throw new IOException("Для общих задач обновите JARVIS на компьютере");JSONObject incoming=request(c,p,outgoing.optBoolean("recovering")?"/v2/recover":"/v2/sync",p.getString("token"),outgoing);if(incoming.optJSONArray("rekey")!=null&&incoming.getJSONArray("rekey").length()>0){SyncJournal.rewrap(c,incoming.getJSONArray("rekey"));continue;}SyncJournal.receive(c,incoming,pull);if(pull)ReminderReceiver.schedule(c);
            if(outgoing.getJSONArray("operations").length()==0&&(!pull||incoming.getJSONArray("items").length()==0)){
                prefs.edit().putString("lastSync",directionTitle(mode)+" · "+MobileStore.now()).putString("lastSyncError","").commit();return;
            }
        }throw new IOException("Журнал большой; повторите обмен для продолжения. Очередь сохранена.");
    }
}
