-- Harbor — Supabase schema (settings persistence)
--
-- Run this in the Supabase SQL editor (Dashboard → SQL → New query) once.
-- It creates the `settings` table + Row-Level Security policies that let a
-- device read/write ONLY its own settings scope. The client talks to this via
-- PostgREST with the anon (publishable) key; RLS is what protects the data, so
-- the anon key being public is safe.
--
-- The client is best-effort: if this schema isn't applied yet, the app keeps
-- working entirely on local storage (no errors, no breakage).
--
-- NOTE: if you previously applied the OLD schema (keyed by `device_id` with
-- auth.uid() policies), drop that table first so this one is created clean:
--   drop table if exists public.settings;

-- The settings table: one row per sync SCOPE. A scope is a stable key shared by
-- BOTH devices in a pair (derived from the two device_ids, sorted), so the two
-- paired devices read/write the SAME row → settings sync across devices. When
-- unpaired, the scope is the device's own id (per-device, no sync).
create table if not exists public.settings (
  scope      text primary key,
  settings   jsonb not null default '{}'::jsonb,
  updated_at timestamptz not null default now()
);

-- Row-Level Security: enable it (no-op if already on) and grant the anon role
-- the minimum needed to read/write through the REST API.
alter table public.settings enable row level security;

-- The client sends the scope in the `X-Scope` request header. PostgREST exposes
-- request headers via current_setting('request.headers', true). A device can
-- read/write only the row whose scope matches the header it sent. If the header
-- is absent, the comparison is `scope = null` → false → access denied (strict).
drop policy if exists "settings_select_own" on public.settings;
create policy "settings_select_own"
  on public.settings
  for select
  using (scope = current_setting('request.headers', true)::jsonb ->> 'x-scope');

drop policy if exists "settings_upsert_own" on public.settings;
create policy "settings_upsert_own"
  on public.settings
  for all
  using (scope = current_setting('request.headers', true)::jsonb ->> 'x-scope')
  with check (scope = current_setting('request.headers', true)::jsonb ->> 'x-scope');

-- Ephemeral voice signaling uses private Supabase Realtime Broadcast channels.
-- The Edge Function must mint a short-lived JWT whose `media_topic` claim is
-- exactly the channel topic and whose `sub` is the Harbor device_id. The public
-- anon key alone never satisfies these policies. SDP/ICE are not stored in a
-- public table; these policies authorize the transient `realtime.messages`
-- records used internally by Broadcast.
--
-- Apply these policies only after enabling Realtime authorization in the project.
-- They are deliberately independent from the settings X-Scope header, which is
-- client-controlled and is not suitable for media authorization.
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
