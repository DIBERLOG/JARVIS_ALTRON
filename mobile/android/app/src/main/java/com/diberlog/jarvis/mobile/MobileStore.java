package com.diberlog.jarvis.mobile;

import android.content.*;
import org.json.*;
import java.time.Instant;
import java.util.*;

/** Same allowlisted Center schema as desktop. No API keys or plaintext vault here. */
final class MobileStore {
    static final String[] LISTS={"notes","reminders","birthdays","habits"};
    static String id(){return UUID.randomUUID().toString();}
    static String now(){return Instant.now().toString();}
    static SharedPreferences prefs(Context c){return c.getSharedPreferences("jarvis-mobile",Context.MODE_PRIVATE);}
    static JSONObject empty() throws JSONException {JSONObject d=new JSONObject();for(String k:LISTS)d.put(k,new JSONArray());return d;}
    static synchronized JSONObject center(Context c) throws JSONException {
        SharedPreferences p=prefs(c);String raw=p.getString("center",null);if(raw!=null){JSONObject d=new JSONObject(raw);validate(d);return d;}
        JSONObject d=empty();String old=p.getString("note","");if(!old.isEmpty())d.getJSONArray("notes").put(new JSONObject().put("id",id()).put("title","Заметка").put("text",old).put("updatedAt",now()).put("format","plain"));return d;
    }
    static synchronized void save(Context c,JSONObject data) throws JSONException {validate(data);if(data.toString().length()>4_000_000)throw new JSONException("Данные больше 4 МБ");if(!prefs(c).edit().putString("center",data.toString()).commit())throw new JSONException("Не удалось сохранить данные");ReminderReceiver.schedule(c);}
    static void validate(JSONObject data) throws JSONException {for(String k:LISTS)records(data.getJSONArray(k));JSONObject t=data.optJSONObject("training");if(t!=null)for(String k:new String[]{"exercises","plans","sessions"})records(t.getJSONArray(k));}
    static void records(JSONArray array)throws JSONException {if(array.length()>10000)throw new JSONException("Слишком много записей");Set<String> ids=new HashSet<>();for(int i=0;i<array.length();i++){String id=array.getJSONObject(i).getString("id");if(id.isEmpty()||id.length()>128||!ids.add(id))throw new JSONException("Некорректный или повторный ID");}}
    static boolean same(Object a,Object b){return canonical(a).equals(canonical(b));}
    static String canonical(Object value){
        if(value==null||value==JSONObject.NULL)return "null";
        if(value instanceof JSONObject){JSONObject o=(JSONObject)value;List<String> keys=new ArrayList<>();o.keys().forEachRemaining(keys::add);Collections.sort(keys);StringBuilder s=new StringBuilder("{");for(String k:keys)s.append(JSONObject.quote(k)).append(':').append(canonical(o.opt(k))).append(',');return s.append('}').toString();}
        if(value instanceof JSONArray){StringBuilder s=new StringBuilder("[");JSONArray a=(JSONArray)value;for(int i=0;i<a.length();i++)s.append(canonical(a.opt(i))).append(',');return s.append(']').toString();}
        if(value instanceof Number)return new java.math.BigDecimal(value.toString()).stripTrailingZeros().toPlainString();
        return value instanceof String?JSONObject.quote((String)value):value.toString();
    }
    static JSONArray mergeRecords(JSONArray base,JSONArray local,JSONArray remote,boolean duplicateConflicts)throws JSONException {
        Map<String,JSONObject>b=map(base),l=map(local),r=map(remote);Set<String> ids=new TreeSet<>();ids.addAll(b.keySet());ids.addAll(l.keySet());ids.addAll(r.keySet());JSONArray result=new JSONArray();
        for(String id:ids){Object bv=b.get(id),lv=l.get(id),rv=r.get(id);Object chosen;
            if(same(lv,rv)||same(rv,bv))chosen=lv;else if(same(lv,bv))chosen=rv;else{chosen=lv;
                if(duplicateConflicts&&rv instanceof JSONObject){JSONObject copy=new JSONObject(rv.toString());copy.put("id",MobileStore.id());copy.put("title",copy.optString("title",copy.optString("name","Запись"))+" · конфликт");result.put(copy);}
                if(duplicateConflicts&&lv==null&&rv!=null)chosen=rv;
            }
            if(chosen!=null&&chosen!=JSONObject.NULL)result.put(chosen);
        }return result;
    }
    static Map<String,JSONObject> map(JSONArray a)throws JSONException{Map<String,JSONObject> m=new LinkedHashMap<>();if(a!=null)for(int i=0;i<a.length();i++){JSONObject o=a.getJSONObject(i);m.put(o.getString("id"),o);}return m;}
    static JSONObject merge(JSONObject base,JSONObject local,JSONObject remote)throws JSONException{
        JSONObject out=new JSONObject(local.toString());for(String k:LISTS)out.put(k,mergeRecords(base.optJSONArray(k),local.optJSONArray(k),remote.optJSONArray(k),true));
        Object b=base.opt("training"),l=local.opt("training"),r=remote.opt("training");
        if(same(l,b))out.put("training",r);
        else if(!same(r,b)&&!same(l,r)&&r instanceof JSONObject&&l instanceof JSONObject){
            JSONObject bt=b instanceof JSONObject?(JSONObject)b:new JSONObject(),lt=(JSONObject)l,rt=(JSONObject)r,merged=new JSONObject(lt.toString());
            for(String k:new String[]{"exercises","plans","sessions"})merged.put(k,mergeRecords(bt.optJSONArray(k),lt.optJSONArray(k),rt.optJSONArray(k),true));
            for(String k:new String[]{"profile","morningExercise","chargeSettings","chargeLevels"}){Object bv=bt.opt(k),lv=lt.opt(k),rv=rt.opt(k);if(same(lv,bv))merged.put(k,rv);else if(!same(rv,bv)&&!same(lv,rv))out.put("phoneTrainingConflict",rt);}
            out.put("training",merged);
        }
        return out;
    }
    static synchronized JSONArray chats(Context c)throws JSONException{return new JSONArray(prefs(c).getString("chats","[]"));}
    static synchronized void saveChats(Context c,JSONArray chats)throws JSONException{records(chats);if(!prefs(c).edit().putString("chats",chats.toString()).commit())throw new JSONException("Не удалось сохранить историю");}
}
