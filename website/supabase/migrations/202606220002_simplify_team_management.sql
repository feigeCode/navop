drop trigger if exists on_auth_user_created on auth.users;
drop function if exists public.handle_new_user();
drop function if exists public.invite_team_member(uuid, text, text);
drop function if exists public.revoke_team_invitation(uuid);

drop table if exists public.team_invitations cascade;
drop table if exists public.audit_events cascade;
drop table if exists public.profiles cascade;

alter table public.team_members add column if not exists email text;

update public.team_members tm
set email = lower(au.email)
from auth.users au
where tm.user_id = au.id and tm.email is null;

update public.team_members
set email = user_id::text
where email is null;

alter table public.team_members alter column email set not null;

create unique index if not exists team_members_team_user_idx
on public.team_members(team_id, user_id);

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
  v_email text;
begin
  select lower(email) into v_email from auth.users where id = auth.uid();

  insert into public.teams (name, description, owner_id)
  values (p_name, p_description, auth.uid())
  returning id into v_team_id;

  insert into public.team_members (team_id, user_id, email, role)
  values (v_team_id, auth.uid(), coalesce(v_email, auth.uid()::text), 'owner');

  return v_team_id;
end;
$$;

create or replace function public.add_team_member_by_email(p_team_id uuid, p_email text, p_role text default 'member')
returns uuid
language plpgsql
security definer
set search_path = public, auth
as $$
declare
  v_email text;
  v_user_id uuid;
  v_member_id uuid;
begin
  v_email := lower(trim(p_email));

  if v_email = '' then
    raise exception 'Email is required';
  end if;

  if p_role = 'owner' and not public.can_manage_team_owners(p_team_id) then
    raise exception 'Only owners can add owners';
  end if;

  if p_role <> 'owner' and not public.can_manage_team_members(p_team_id) then
    raise exception 'Insufficient team permissions';
  end if;

  select id into v_user_id from auth.users where lower(email) = v_email limit 1;

  if v_user_id is null then
    raise exception 'User does not exist. Ask them to sign in first.';
  end if;

  insert into public.team_members (team_id, user_id, email, role)
  values (p_team_id, v_user_id, v_email, p_role)
  on conflict (team_id, user_id) do update set
    email = excluded.email,
    role = excluded.role
  returning id into v_member_id;

  return v_member_id;
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
end;
$$;
