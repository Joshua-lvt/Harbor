-- Harbor — Supabase schema (settings persistence)
--
-- Run this in the Supabase SQL editor (Dashboard → SQL → New query) once.
-- It creates the `settings` table + Row-Level Security policies that let a
-- device read/write ONLY its own row, keyed by device_id. The client talks to
-- this via PostgREST with the anon (publishable) key; RLS is what protects the
-- data, so the anon key being public is safe.
--
-- The client is best-effort: if this schema isn't applied yet, the app keeps
-- working entirely on local storage (no errors, no breakage).

-- The settings table: one row per device, holding the user's preferences as a
-- JSONB blob (mirrors the local Settings shape in lib/types.ts).
create table if not exists public.settings (
  device_id  text primary key,
  settings   jsonb not null default '{}'::jsonb,
  updated_at timestamptz not null default now()
);

-- Row-Level Security: enable it (no-op if already on) and grant the anon role
-- the minimum needed to read/write through the REST API.
alter table public.settings enable row level security;

-- A device can read its own row.
drop policy if exists "settings_select_own" on public.settings;
create policy "settings_select_own"
  on public.settings
  for select
  using (device_id = auth.uid()::text);

-- A device can insert/update its own row.
drop policy if exists "settings_upsert_own" on public.settings;
create policy "settings_upsert_own"
  on public.settings
  for all
  using (device_id = auth.uid()::text)
  with check (device_id = auth.uid()::text);

-- NOTE on auth.uid(): the client currently sends device_id as a plain column
-- value (no Supabase auth session), so auth.uid() is null and these policies
-- would block access. To make RLS work with the anon key you have two options:
--
--   (A) Simplest for a 2-person app: replace `auth.uid()::text` with the
--       device_id the client sends, e.g. using a request header. PostgREST
--       exposes custom headers as `current_setting('request.headers', true)`.
--       Example policy:
--         using (device_id = coalesce(
--           nullif(current_setting('request.headers', true)::jsonb ->> 'x-device-id', ''),
--           device_id
--         ))
--       and have the client send `X-Device-Id` (see supabase.ts).
--
--   (B) Or, for a private couple app, keep RLS off on this table and rely on
--       the anon key being unguessable-in-practice. Not recommended.
--
-- The client (supabase.ts) is written to send an `X-Device-Id` header; pick
-- option (A) and uncomment the header in supabase.ts to enforce per-device
-- isolation. Until then the table is created but the policies above are the
-- strict (auth.uid()) form, which will block writes — so either apply (A) or
-- temporarily disable RLS to test.
