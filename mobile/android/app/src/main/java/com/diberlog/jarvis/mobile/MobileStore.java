package com.diberlog.jarvis.mobile;

import android.content.*;
import org.json.*;
import java.time.Instant;
import java.util.*;

/** Same allowlisted Center schema as desktop. No API keys or plaintext vault here. */
final class MobileStore {
    private static final Map<Object,SyncJournal.Edit> EDITS=new IdentityEdits();
    // Android JSONArray has a mutable value hash: ordinary WeakHashMap loses its edit baseline after mutation.
    private static final class IdentityEdits extends AbstractMap<Object,SyncJournal.Edit>{
        final java.lang.ref.ReferenceQueue<Object> queue=new java.lang.ref.ReferenceQueue<>();final Map<Key,SyncJournal.Edit> values=new HashMap<>();
        static final class Key extends java.lang.ref.WeakReference<Object>{final int hash;Key(Object value,java.lang.ref.ReferenceQueue<Object> queue){super(value,queue);hash=System.identityHashCode(value);}public int hashCode(){return hash;}public boolean equals(Object other){return this==other||other instanceof Key&&get()!=null&&get()==((Key)other).get();}}
        void clean(){java.lang.ref.Reference<?> expired;while((expired=queue.poll())!=null)values.remove(expired);}
        public SyncJournal.Edit get(Object value){clean();return values.get(new Key(value,null));}
        public SyncJournal.Edit put(Object value,SyncJournal.Edit edit){clean();return values.put(new Key(value,queue),edit);}
        public Set<Map.Entry<Object,SyncJournal.Edit>> entrySet(){clean();Set<Map.Entry<Object,SyncJournal.Edit>> result=new HashSet<>();for(Map.Entry<Key,SyncJournal.Edit> entry:values.entrySet()){Object value=entry.getKey().get();if(value!=null)result.add(new SimpleImmutableEntry<>(value,entry.getValue()));}return result;}
    }
    static final String[] LISTS={"notes","reminders","birthdays","habits","tasks"};
    static String id(){return UUID.randomUUID().toString();}
    static String now(){return Instant.now().toString();}
    static SharedPreferences prefs(Context c){return c.getSharedPreferences("jarvis-mobile",Context.MODE_PRIVATE);}
    static JSONObject empty() throws JSONException {JSONObject d=new JSONObject();for(String k:LISTS)d.put(k,new JSONArray());return d;}
    static synchronized JSONObject center(Context c) throws JSONException {
        try{SyncJournal.Edit edit=SyncJournal.editState(c);JSONObject center=new JSONObject(edit.snapshot.getString("center_data"));if(!center.has("tasks"))center.put("tasks",new JSONArray());EDITS.put(center,edit);for(String list:LISTS){JSONArray items=center.getJSONArray(list);for(int i=0;i<items.length();i++)EDITS.put(items.getJSONObject(i),edit);}return center;}catch(Exception e){throw new JSONException(e.getMessage());}
    }
    static synchronized void save(Context c,JSONObject data) throws JSONException {
        validate(data);if(data.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8).length>16*1024*1024)throw new JSONException("Данные больше 16 МБ");
        try{SyncJournal.saveEdit(c,"center_data",data.toString(),EDITS.get(data));}catch(Exception e){throw new JSONException(e.getMessage());}ReminderReceiver.schedule(c);
    }
    static synchronized JSONObject editCenter(Context c,JSONObject record)throws JSONException{SyncJournal.Edit edit=EDITS.get(record);if(edit==null)return center(c);JSONObject center=new JSONObject(edit.snapshot.getString("center_data"));if(!center.has("tasks"))center.put("tasks",new JSONArray());EDITS.put(center,edit);return center;}
    static void validate(JSONObject data) throws JSONException {if(!data.has("tasks"))data.put("tasks",new JSONArray());for(String k:LISTS)records(data.getJSONArray(k));JSONArray notes=data.getJSONArray("notes");for(int n=0;n<notes.length();n++)if(notes.getJSONObject(n).optString("type").equals("mindmap"))MindMapData.validate(notes.getJSONObject(n).getJSONObject("mindMap"));JSONArray reminders=data.getJSONArray("reminders");for(int n=0;n<reminders.length();n++){JSONObject r=reminders.getJSONObject(n);if(r.has("repeat")&&!r.optString("repeat").equals("daily")&&!r.optString("repeat").equals("none"))throw new JSONException("Неизвестный режим повторения напоминания");}JSONArray tasks=data.getJSONArray("tasks");for(int n=0;n<tasks.length();n++){JSONObject task=tasks.getJSONObject(n);if(task.getString("text").trim().isEmpty()||task.getString("text").length()>1000||!(task.get("done") instanceof Boolean))throw new JSONException("Некорректная задача");}JSONObject t=data.optJSONObject("training");if(t!=null)for(String k:new String[]{"exercises","plans","sessions"})records(t.getJSONArray(k));}
    static void records(JSONArray array)throws JSONException {if(array.length()>10000)throw new JSONException("Слишком много записей");Set<String> ids=new HashSet<>();for(int i=0;i<array.length();i++){if(array.getJSONObject(i).toString().getBytes(java.nio.charset.StandardCharsets.UTF_8).length>4*1024*1024)throw new JSONException("Запись больше 4 МБ. Уменьшите картинки или объём записи.");String id=array.getJSONObject(i).getString("id");if(id.isEmpty()||id.length()>128||!ids.add(id))throw new JSONException("Некорректный или повторный ID");}}
    static boolean same(Object a,Object b){return canonical(a).equals(canonical(b));}
    static String canonical(Object value){
        if(value==null||value==JSONObject.NULL)return "null";
        if(value instanceof JSONObject){JSONObject o=(JSONObject)value;List<String> keys=new ArrayList<>();o.keys().forEachRemaining(keys::add);Collections.sort(keys);StringBuilder s=new StringBuilder("{");for(String k:keys)s.append(JSONObject.quote(k)).append(':').append(canonical(o.opt(k))).append(',');return s.append('}').toString();}
        if(value instanceof JSONArray){StringBuilder s=new StringBuilder("[");JSONArray a=(JSONArray)value;for(int i=0;i<a.length();i++)s.append(canonical(a.opt(i))).append(',');return s.append(']').toString();}
        if(value instanceof Number)return new java.math.BigDecimal(value.toString()).stripTrailingZeros().toPlainString();
        return value instanceof String?JSONObject.quote((String)value):value.toString();
    }
    static JSONArray mergeChatsStrict(JSONArray base,JSONArray local,JSONArray remote)throws JSONException{Map<String,JSONObject>b=map(base),l=map(local),r=map(remote);Set<String> ids=new HashSet<>();ids.addAll(b.keySet());ids.addAll(l.keySet());ids.addAll(r.keySet());for(String id:ids){Object before=b.get(id),current=l.get(id),requested=r.get(id);if(!same(current,requested)&&!same(requested,before)&&!same(current,before))throw new JSONException("Эта беседа изменена другим клиентом. Исходная история сохранена; обновите чат.");}return mergeRecords(base,local,remote,false);}
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
    static synchronized JSONArray chats(Context c)throws JSONException{try{SyncJournal.Edit edit=SyncJournal.editState(c);JSONArray chats=new JSONArray(edit.snapshot.getString("chat_history"));EDITS.put(chats,edit);for(int n=0;n<chats.length();n++)EDITS.put(chats.getJSONObject(n),edit);return chats;}catch(Exception e){throw new JSONException(e.getMessage());}}
    static synchronized void saveChat(Context c,JSONObject chat)throws JSONException{try{synchronized(SyncJournal.class){SyncJournal.Edit base=EDITS.get(chat);JSONArray chats=base==null?chats(c):new JSONArray(base.snapshot.getString("chat_history"));if(base!=null)EDITS.put(chats,base);boolean found=false;for(int n=0;n<chats.length();n++)if(chats.getJSONObject(n).getString("id").equals(chat.getString("id"))){chats.put(n,chat);found=true;break;}if(!found)chats.put(chat);saveChats(c,chats);EDITS.put(chat,SyncJournal.editState(c));}}catch(Exception e){throw new JSONException(e.getMessage());}}
    static synchronized void saveChats(Context c,JSONArray chats)throws JSONException{records(chats);try{SyncJournal.saveEdit(c,"chat_history",chats.toString(),EDITS.get(chats));}catch(Exception e){throw new JSONException(e.getMessage());}}    static synchronized JSONArray localTasks(Context c)throws JSONException{try{return new JSONArray(SyncJournal.snapshot(c).optString("local_tasks","[]"));}catch(Exception e){throw new JSONException(e.getMessage());}}
    static synchronized void saveLocalTasks(Context c,JSONArray tasks)throws JSONException{try{if(tasks.toString().length()>4_000_000)throw new Exception("Список задач слишком большой");SyncJournal.save(c,"local_tasks",tasks.toString());}catch(Exception e){throw new JSONException(e.getMessage());}}

}
