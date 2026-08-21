-- Harbor private voice signaling authorization.
-- Broadcast payloads are transient; only the two JWT-authorized peers may
-- read or publish messages for the exact room topic in media_topic.

drop policy if exists "voice_room_members_receive" on realtime.messages;
create policy "voice_room_members_receive"
  on realtime.messages
  for select
  to authenticated
  using (
    realtime.topic() = (select auth.jwt() ->> 'media_topic')
  );

drop policy if exists "voice_room_members_publish" on realtime.messages;
create policy "voice_room_members_publish"
  on realtime.messages
  for insert
  to authenticated
  with check (
    realtime.topic() = (select auth.jwt() ->> 'media_topic')
    and payload ->> 'sender_id' = (select auth.jwt() ->> 'sub')
  );
