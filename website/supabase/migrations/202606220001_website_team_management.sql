create extension if not exists pgcrypto;

create table if not exists public.teams (
  id uuid primary key default gen_random_uuid(),
  name text not null,
  owner_id uuid not null references auth.users(id) on delete cascade,
  description text,
  key_verification text,
  key_version integer not null default 1,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.team_members (
  id uuid primary key default gen_random_uuid(),
  team_id uuid not null references public.teams(id) on delete cascade,
  user_id uuid not null references auth.users(id) on delete cascade,
  role text not null default 'member',
  joined_at timestamptz not null default now()
);

alter table public.teams add column if not exists description text;
alter table public.teams add column if not exists key_verification text;
alter table public.teams add column if not exists key_version integer not null default 1;
alter table public.teams add column if not exists created_at timestamptz not null default now();
alter table public.teams add column if not exists updated_at timestamptz not null default now();
alter table public.team_members add column if not exists role text not null default 'member';
alter table public.team_members add column if not exists joined_at timestamptz not null default now();

create table if not exists public.profiles (
  id uuid primary key references auth.users(id) on delete cascade,
  email text,
  display_name text,
  avatar_url text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.team_invitations (
  id uuid primary key default gen_random_uuid(),
  team_id uuid not null references public.teams(id) on delete cascade,
  email text not null,
  role text not null default 'member',
  invited_by uuid not null references auth.users(id) on delete cascade,
  accepted_at timestamptz,
  revoked_at timestamptz,
  expires_at timestamptz not null default now() + interval '7 days',
  created_at timestamptz not null default now(),
  constraint team_invitations_role_check check (role in ('owner', 'admin', 'member'))
);

create table if not exists public.audit_events (
  id uuid primary key default gen_random_uuid(),
  team_id uuid references public.teams(id) on delete cascade,
  actor_id uuid references auth.users(id) on delete set null,
  event_type text not null,
  target_id uuid,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

alter table public.team_members drop constraint if exists team_members_role_check;
alter table public.team_members
  add constraint team_members_role_check check (role in ('owner', 'admin', 'member'));

create unique index if not exists team_members_team_user_idx
on public.team_members(team_id, user_id);

create or replace function public.handle_new_user()
returns trigger
language plpgsql
security definer
set search_path = public
as $$
begin
  insert into public.profiles (id, email, display_name, avatar_url)
  values (
    new.id,
    new.email,
    coalesce(new.raw_user_meta_data ->> 'display_name', split_part(new.email, '@', 1)),
    new.raw_user_meta_data ->> 'avatar_url'
  )
  on conflict (id) do update set
    email = excluded.email,
    display_name = coalesce(public.profiles.display_name, excluded.display_name),
    avatar_url = coalesce(public.profiles.avatar_url, excluded.avatar_url),
    updated_at = now();
  return new;
end;
$$;

drop trigger if exists on_auth_user_created on auth.users;
create trigger on_auth_user_created
after insert on auth.users
for each row execute function public.handle_new_user();

insert into public.profiles (id, email, display_name)
select id, email, split_part(email, '@', 1)
from auth.users
on conflict (id) do nothing;

create or replace function public.current_team_role(p_team_id uuid)
returns text
language sql
stable
security definer
set search_path = public
as $$
  select tm.role
  from public.team_members tm
  where tm.team_id = p_team_id and tm.user_id = auth.uid()
  limit 1
$$;

create or replace function public.can_manage_team_members(p_team_id uuid)
returns boolean
language sql
stable
security definer
set search_path = public
as $$
  select coalesce(public.current_team_role(p_team_id) in ('owner', 'admin'), false)
$$;

create or replace function public.can_manage_team_owners(p_team_id uuid)
returns boolean
language sql
stable
security definer
set search_path = public
as $$
  select coalesce(public.current_team_role(p_team_id) = 'owner', false)
$$;

create or replace function public.create_team_with_owner(p_name text, p_description text default null)
returns uuid
language plpgsql
security definer
set search_path = public
as $$
declare
  v_team_id uuid;
begin
  insert into public.teams (name, description, owner_id)
  values (p_name, p_description, auth.uid())
  returning id into v_team_id;

  insert into public.team_members (team_id, user_id, role)
  values (v_team_id, auth.uid(), 'owner');

  insert into public.audit_events (team_id, actor_id, event_type)
  values (v_team_id, auth.uid(), 'team.created');

  return v_team_id;
end;
$$;

create or replace function public.invite_team_member(p_team_id uuid, p_email text, p_role text default 'member')
returns uuid
language plpgsql
security definer
set search_path = public, auth
as $$
declare
  v_user_id uuid;
  v_invitation_id uuid;
begin
  if p_role = 'owner' and not public.can_manage_team_owners(p_team_id) then
    raise exception 'Only owners can invite owners';
  end if;

  if p_role <> 'owner' and not public.can_manage_team_members(p_team_id) then
    raise exception 'Insufficient team permissions';
  end if;

  select id into v_user_id from auth.users where lower(email) = lower(p_email) limit 1;

  if v_user_id is not null then
    insert into public.team_members (team_id, user_id, role)
    values (p_team_id, v_user_id, p_role)
    on conflict (team_id, user_id) do update set role = excluded.role
    returning id into v_invitation_id;
  else
    insert into public.team_invitations (team_id, email, role, invited_by)
    values (p_team_id, lower(p_email), p_role, auth.uid())
    returning id into v_invitation_id;
  end if;

  insert into public.audit_events (team_id, actor_id, event_type, metadata)
  values (p_team_id, auth.uid(), 'member.invited', jsonb_build_object('email', p_email, 'role', p_role));

  return v_invitation_id;
end;
$$;

create or replace function public.update_team_member_role(p_member_id uuid, p_role text)
returns void
language plpgsql
security definer
set search_path = public
as $$
declare
  v_team_id uuid;
  v_old_role text;
begin
  select team_id, role into v_team_id, v_old_role
  from public.team_members where id = p_member_id;

  if v_team_id is null then
    raise exception 'Member not found';
  end if;

  if v_old_role = 'owner' and not public.can_manage_team_owners(v_team_id) then
    raise exception 'Only owners can manage owners';
  end if;

  if p_role = 'owner' and not public.can_manage_team_owners(v_team_id) then
    raise exception 'Only owners can assign owners';
  end if;

  if p_role <> 'owner' and not public.can_manage_team_members(v_team_id) then
    raise exception 'Insufficient team permissions';
  end if;

  update public.team_members set role = p_role where id = p_member_id;
  insert into public.audit_events (team_id, actor_id, event_type, target_id, metadata)
  values (v_team_id, auth.uid(), 'member.role_updated', p_member_id, jsonb_build_object('role', p_role));
end;
$$;

create or replace function public.remove_team_member(p_member_id uuid)
returns void
language plpgsql
security definer
set search_path = public
as $$
declare
  v_team_id uuid;
  v_role text;
begin
  select team_id, role into v_team_id, v_role
  from public.team_members where id = p_member_id;

  if v_role = 'owner' then
    raise exception 'Owners cannot be removed from the website console';
  end if;

  if v_role = 'member' and not public.can_manage_team_members(v_team_id) then
    raise exception 'Insufficient team permissions';
  end if;

  if v_role = 'admin' and not public.can_manage_team_owners(v_team_id) then
    raise exception 'Only owners can remove admins';
  end if;

  delete from public.team_members where id = p_member_id;
  insert into public.audit_events (team_id, actor_id, event_type, target_id)
  values (v_team_id, auth.uid(), 'member.removed', p_member_id);
end;
$$;

alter table public.profiles enable row level security;
alter table public.teams enable row level security;
alter table public.team_members enable row level security;
alter table public.team_invitations enable row level security;
alter table public.audit_events enable row level security;

drop policy if exists profiles_select_team_visible on public.profiles;
create policy profiles_select_team_visible on public.profiles
for select using (
  id = auth.uid()
  or exists (
    select 1 from public.team_members mine
    join public.team_members theirs on theirs.team_id = mine.team_id
    where mine.user_id = auth.uid() and theirs.user_id = profiles.id
  )
);

drop policy if exists profiles_update_self on public.profiles;
create policy profiles_update_self on public.profiles
for update using (id = auth.uid()) with check (id = auth.uid());

drop policy if exists teams_select_members on public.teams;
create policy teams_select_members on public.teams
for select using (public.current_team_role(id) is not null);

drop policy if exists teams_update_managers on public.teams;
create policy teams_update_managers on public.teams
for update using (public.can_manage_team_members(id)) with check (public.can_manage_team_members(id));

drop policy if exists team_members_select_team_members on public.team_members;
create policy team_members_select_team_members on public.team_members
for select using (public.current_team_role(team_id) is not null);

drop policy if exists team_invitations_select_managers on public.team_invitations;
create policy team_invitations_select_managers on public.team_invitations
for select using (public.can_manage_team_members(team_id));

drop policy if exists team_invitations_insert_managers on public.team_invitations;
create policy team_invitations_insert_managers on public.team_invitations
for insert with check (public.can_manage_team_members(team_id));

drop policy if exists audit_events_select_members on public.audit_events;
create policy audit_events_select_members on public.audit_events
for select using (public.current_team_role(team_id) is not null);
