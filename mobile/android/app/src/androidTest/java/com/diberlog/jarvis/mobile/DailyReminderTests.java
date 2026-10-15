package com.diberlog.jarvis.mobile;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.json.*;
import java.time.*;
import java.util.*;
import static org.junit.Assert.*;
@RunWith(AndroidJUnit4.class)
public class DailyReminderTests {
    JSONObject reminder(String at)throws Exception{return new JSONObject().put("id","daily-test").put("title","Daily").put("dueAt",at).put("createdAt",at).put("repeat","daily").put("done",false);}
    JSONObject data(JSONObject r)throws Exception{return MobileStore.empty().put("reminders",new JSONArray().put(r));}
    long at(String value){return LocalDateTime.parse(value).atZone(ZoneId.systemDefault()).toInstant().toEpochMilli();}
    @Test public void repeatsAndSchedulesTomorrowWithoutReplayingOldDays()throws Exception{
        JSONObject r=reminder("2026-10-01T09:00:00");List<ReminderReceiver.Event> events=ReminderReceiver.events(data(r),at("2026-10-15T10:00:00"));
        assertEquals(2,events.size());assertEquals(at("2026-10-15T09:00:00"),events.get(0).when);assertEquals(at("2026-10-16T09:00:00"),events.get(1).when);
        assertTrue(ReminderReceiver.occurrence(r,events.get(1).when,0));assertFalse(ReminderReceiver.occurrence(r,at("2026-10-16T10:00:00"),0));assertFalse(ReminderReceiver.occurrence(r,events.get(1).when,1));
        r.put("done",true);assertTrue(ReminderReceiver.events(data(r),at("2026-10-15T10:00:00")).isEmpty());
    }
    @Test public void creationAndFutureStartDoNotNotifyEarly()throws Exception{
        JSONObject r=reminder("2026-10-01T09:00:00").put("createdAt","2026-10-15T10:00:00");List<ReminderReceiver.Event> events=ReminderReceiver.events(data(r),at("2026-10-15T10:01:00"));assertEquals(1,events.size());assertEquals(at("2026-10-16T09:00:00"),events.get(0).when);
        r=reminder("2026-11-01T09:00:00");events=ReminderReceiver.events(data(r),at("2026-10-15T10:00:00"));assertEquals(1,events.size());assertEquals(at("2026-11-01T09:00:00"),events.get(0).when);
    }
    @Test public void calendarDayPreservesClockAcrossDaylightSaving()throws Exception{
        TimeZone previous=TimeZone.getDefault();try{TimeZone.setDefault(TimeZone.getTimeZone("Europe/Berlin"));JSONObject r=reminder("2026-10-24T09:00:00");List<ReminderReceiver.Event> events=ReminderReceiver.events(data(r),at("2026-10-24T10:00:00"));assertEquals(at("2026-10-25T09:00:00"),events.get(1).when);assertEquals(25*3600000L,events.get(1).when-events.get(0).when);}finally{TimeZone.setDefault(previous);}
    }
    @Test public void springGapDoesNotShiftFollowingDays()throws Exception{
        TimeZone previous=TimeZone.getDefault();try{TimeZone.setDefault(TimeZone.getTimeZone("Europe/Berlin"));JSONObject r=reminder("2026-03-28T02:30:00");List<ReminderReceiver.Event> events=ReminderReceiver.events(data(r),at("2026-03-29T10:00:00"));assertEquals(at("2026-03-30T02:30:00"),events.get(1).when);assertTrue(ReminderReceiver.occurrence(r,events.get(0).when,0));}finally{TimeZone.setDefault(previous);}
    }
    @Test public void syncMergePreservesDailyAndCanReturnToOnce()throws Exception{
        JSONObject base=data(reminder("2026-10-15T09:00:00"));JSONObject remote=new JSONObject(base.toString());remote.getJSONArray("reminders").getJSONObject(0).put("repeat","none");JSONObject merged=MobileStore.merge(base,base,remote);assertEquals("none",merged.getJSONArray("reminders").getJSONObject(0).getString("repeat"));MobileStore.validate(merged);
    }
}
