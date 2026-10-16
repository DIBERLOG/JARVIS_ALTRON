package com.diberlog.jarvis.mobile;
import org.json.*;
import java.util.*;
import java.nio.charset.StandardCharsets;
/** Validate the shared note payload before local writes, restore and synchronization. */
final class MindMapData {
 static void text(JSONObject o,String key,int limit)throws JSONException{Object v=o.get(key);if(!(v instanceof String)||((String)v).length()>limit)throw new JSONException("Некорректное поле карты: "+key);}
 static Set<String> ids(JSONArray list,int maximum)throws JSONException{if(list.length()>maximum)throw new JSONException("Слишком много элементов карты");Set<String> ids=new HashSet<>();for(int n=0;n<list.length();n++){JSONObject item=list.getJSONObject(n);text(item,"id",128);String id=item.getString("id");if(id.isEmpty()||!ids.add(id))throw new JSONException("Повторный ID карты");}return ids;}
 static void validate(JSONObject map)throws JSONException{
  if(map.getInt("version")!=1||map.toString().getBytes(StandardCharsets.UTF_8).length>3_400_000)throw new JSONException("Неизвестная версия карты или превышен размер");
  JSONArray nodes=map.getJSONArray("nodes"),edges=map.getJSONArray("edges");Set<String> ids=ids(nodes,80);ids(edges,160);if(nodes.length()==0)throw new JSONException("Карта пуста");
  for(int n=0;n<nodes.length();n++){JSONObject node=nodes.getJSONObject(n);text(node,"title",120);text(node,"text",4000);if(!Arrays.asList("idea","goal","plan","images","note").contains(node.getString("kind")))throw new JSONException("Неизвестная карточка");for(String axis:new String[]{"x","y"}){double v=node.getDouble(axis);if(!Double.isFinite(v)||Math.abs(v)>10000)throw new JSONException("Неверные координаты");}
   for(String key:new String[]{"width","height","imageHeight"})if(node.has(key)){double v=node.getDouble(key),min=key.equals("width")?240:key.equals("height")?160:80,max=key.equals("width")?1000:key.equals("height")?1400:800;if(!Double.isFinite(v)||v<min||v>max)throw new JSONException("Некорректный размер карточки");}if(node.has("imageFit")&&!Arrays.asList("contain","cover").contains(node.getString("imageFit")))throw new JSONException("Некорректный вид изображения");
   JSONArray items=node.getJSONArray("items"),images=node.getJSONArray("images"),files=node.getJSONArray("files");ids(items,100);ids(images,8);ids(files,8);
   for(int j=0;j<items.length();j++){JSONObject i=items.getJSONObject(j);text(i,"text",1000);if(!(i.get("done") instanceof Boolean))throw new JSONException("Неверная отметка плана");}
   for(int j=0;j<images.length();j++){JSONObject i=images.getJSONObject(j);text(i,"name",160);text(i,"src",2_800_000);if(!i.getString("src").matches("^data:image/(jpeg|png|webp|gif);base64,[a-zA-Z0-9+/]+={0,2}$"))throw new JSONException("Неверное изображение");}
   for(int j=0;j<files.length();j++){JSONObject f=files.getJSONObject(j);text(f,"name",160);text(f,"data",940000);String raw=f.getString("data");if(raw.length()%4!=0||!raw.matches("^[A-Za-z0-9+/]+={0,2}$")||!Arrays.asList("application/pdf","text/plain","text/markdown","application/vnd.openxmlformats-officedocument.wordprocessingml.document").contains(f.getString("mime")))throw new JSONException("Неверное вложение");int size=f.getInt("size");if(size<1||size>700000||android.util.Base64.decode(raw,android.util.Base64.NO_WRAP).length!=size)throw new JSONException("Вложение повреждено");}
  }
  Set<String> pairs=new HashSet<>();for(int n=0;n<edges.length();n++){JSONObject edge=edges.getJSONObject(n);String from=edge.getString("from"),to=edge.getString("to");if(!ids.contains(from)||!ids.contains(to)||from.equals(to)||!pairs.add(from.compareTo(to)<0?from+":"+to:to+":"+from))throw new JSONException("Некорректная связь карты");}
 }
}
