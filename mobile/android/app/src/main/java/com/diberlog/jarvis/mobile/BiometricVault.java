package com.diberlog.jarvis.mobile;

import android.security.keystore.*;
import androidx.biometric.*;
import androidx.fragment.app.FragmentActivity;
import javax.crypto.*;
import javax.crypto.spec.*;
import java.security.*;
import java.nio.charset.StandardCharsets;
import org.json.*;

final class BiometricVault {
    interface Result {void done(SecretKey key);void error(String message);}
    static final String ALIAS="jarvis-vault-biometric-v1";
    static String binding(JSONObject f){return MobileStore.canonical(f.opt("passwordKey"))+f.optString("salt");}
    static void authenticate(FragmentActivity activity,JSONObject vault,SecretKey existing,Result callback){
        if(BiometricManager.from(activity).canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG)!=BiometricManager.BIOMETRIC_SUCCESS){callback.error("Сильная биометрия недоступна. Используйте мастер-пароль.");return;}
        try{
            KeyStore store=KeyStore.getInstance("AndroidKeyStore");store.load(null);boolean enable=existing!=null;
            if(enable){if(store.containsAlias(ALIAS))store.deleteEntry(ALIAS);KeyGenerator gen=KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES,"AndroidKeyStore");gen.init(new KeyGenParameterSpec.Builder(ALIAS,KeyProperties.PURPOSE_ENCRYPT|KeyProperties.PURPOSE_DECRYPT).setBlockModes(KeyProperties.BLOCK_MODE_GCM).setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setKeySize(256).setUserAuthenticationRequired(true).setInvalidatedByBiometricEnrollment(true).build());gen.generateKey();}
            SecretKey device=(SecretKey)store.getKey(ALIAS,null);if(device==null)throw new GeneralSecurityException();
            Cipher cipher=Cipher.getInstance("AES/GCM/NoPadding");JSONObject sealed=enable?null:new JSONObject(MobileStore.prefs(activity).getString("vaultBiometric","{}"));
            if(enable)cipher.init(Cipher.ENCRYPT_MODE,device);else cipher.init(Cipher.DECRYPT_MODE,device,new GCMParameterSpec(128,VaultCrypto.bytes(sealed.getString("iv"))));
            cipher.updateAAD(binding(vault).getBytes(StandardCharsets.UTF_8));
            BiometricPrompt prompt=new BiometricPrompt(activity,activity::runOnUiThread,new BiometricPrompt.AuthenticationCallback(){
                @Override public void onAuthenticationSucceeded(BiometricPrompt.AuthenticationResult result){byte[] raw=null;try{Cipher c=result.getCryptoObject().getCipher();if(enable){raw=existing.getEncoded();JSONObject out=new JSONObject().put("iv",VaultCrypto.b64(c.getIV())).put("data",VaultCrypto.b64(c.doFinal(raw)));MobileStore.prefs(activity).edit().putString("vaultBiometric",out.toString()).apply();callback.done(existing);}else{raw=c.doFinal(VaultCrypto.bytes(sealed.getString("data")));SecretKey key=new SecretKeySpec(raw,"AES");VaultCrypto.cards(vault,key);callback.done(key);}}catch(Exception e){callback.error("Не удалось разблокировать сейф. Используйте мастер-пароль и настройте биометрию заново.");}finally{if(raw!=null)java.util.Arrays.fill(raw,(byte)0);}}
                @Override public void onAuthenticationError(int code,CharSequence message){callback.error("Биометрическая разблокировка отменена или недоступна.");}
            });
            prompt.authenticate(new BiometricPrompt.PromptInfo.Builder().setTitle(enable?"Разрешить отпечаток для сейфа":"Разблокировать сейф").setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG).setNegativeButtonText("Мастер-пароль").build(),new BiometricPrompt.CryptoObject(cipher));
        }catch(Exception e){callback.error("Биометрический ключ недоступен. Разблокируйте мастер-паролем и настройте заново.");}
    }
}
