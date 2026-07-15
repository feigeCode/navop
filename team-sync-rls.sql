BEGIN;

-- 统一 team_members.role 为 owner/admin/member。
DO $$
DECLARE
    constraint_record RECORD;
BEGIN
    FOR constraint_record IN
        SELECT c.conname
        FROM pg_constraint c
        JOIN pg_class t ON t.oid = c.conrelid
        JOIN pg_namespace n ON n.oid = t.relnamespace
        WHERE n.nspname = 'public'
          AND t.relname = 'team_members'
          AND c.contype = 'c'
          AND pg_get_constraintdef(c.oid) ILIKE '%role%'
    LOOP
        EXECUTE format(
            'ALTER TABLE public.team_members DROP CONSTRAINT IF EXISTS %I',
            constraint_record.conname
        );
    END LOOP;
END;
$$;

ALTER TABLE public.team_members
    ADD CONSTRAINT team_members_role_check
    CHECK (role IN ('owner', 'admin', 'member'));

-- SECURITY DEFINER 用于避免 team_members RLS 自递归。
CREATE OR REPLACE FUNCTION public.current_team_role(p_team_id uuid)
RETURNS text
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT tm.role
    FROM public.team_members tm
    WHERE tm.team_id = p_team_id AND tm.user_id = auth.uid()
    LIMIT 1;
$$;

CREATE OR REPLACE FUNCTION public.get_my_team_ids()
RETURNS SETOF uuid
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT tm.team_id
    FROM public.team_members tm
    WHERE tm.user_id = auth.uid();
$$;

CREATE OR REPLACE FUNCTION public.can_manage_team_members(p_team_id uuid)
RETURNS boolean
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT COALESCE(
        public.current_team_role(p_team_id) IN ('owner', 'admin'), false
    );
$$;

CREATE OR REPLACE FUNCTION public.can_manage_team_owners(p_team_id uuid)
RETURNS boolean
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT COALESCE(public.current_team_role(p_team_id) = 'owner', false);
$$;

CREATE OR REPLACE FUNCTION public.can_edit_team_sync_data(p_team_id uuid)
RETURNS boolean
LANGUAGE sql
STABLE
SECURITY DEFINER
SET search_path = public
AS $$
    SELECT COALESCE(
        public.current_team_role(p_team_id) IN ('owner', 'admin'), false
    );
$$;

-- owner_id 不能通过普通 UPDATE 转移；团队 admin 只能编辑数据，不能夺取所有权。
CREATE OR REPLACE FUNCTION public.prevent_direct_owner_change()
RETURNS trigger
LANGUAGE plpgsql
SET search_path = public
AS $$
BEGIN
    IF NEW.owner_id IS DISTINCT FROM OLD.owner_id THEN
        RAISE EXCEPTION 'Record owner cannot be changed directly'
            USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS trg_prevent_direct_team_owner_change ON public.teams;
CREATE TRIGGER trg_prevent_direct_team_owner_change
BEFORE UPDATE OF owner_id ON public.teams
FOR EACH ROW
EXECUTE FUNCTION public.prevent_direct_owner_change();

DROP TRIGGER IF EXISTS trg_prevent_direct_sync_data_owner_change ON public.sync_data;
CREATE TRIGGER trg_prevent_direct_sync_data_owner_change
BEFORE UPDATE OF owner_id ON public.sync_data
FOR EACH ROW
EXECUTE FUNCTION public.prevent_direct_owner_change();

REVOKE ALL ON FUNCTION public.current_team_role(uuid) FROM PUBLIC, anon;
REVOKE ALL ON FUNCTION public.get_my_team_ids() FROM PUBLIC, anon;
REVOKE ALL ON FUNCTION public.can_manage_team_members(uuid) FROM PUBLIC, anon;
REVOKE ALL ON FUNCTION public.can_manage_team_owners(uuid) FROM PUBLIC, anon;
REVOKE ALL ON FUNCTION public.can_edit_team_sync_data(uuid) FROM PUBLIC, anon;
REVOKE ALL ON FUNCTION public.prevent_direct_owner_change()
FROM PUBLIC, anon, authenticated, service_role;
GRANT EXECUTE ON FUNCTION public.current_team_role(uuid) TO authenticated, service_role;
GRANT EXECUTE ON FUNCTION public.get_my_team_ids() TO authenticated, service_role;
GRANT EXECUTE ON FUNCTION public.can_manage_team_members(uuid) TO authenticated, service_role;
GRANT EXECUTE ON FUNCTION public.can_manage_team_owners(uuid) TO authenticated, service_role;
GRANT EXECUTE ON FUNCTION public.can_edit_team_sync_data(uuid) TO authenticated, service_role;

ALTER TABLE public.sync_data ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.teams ENABLE ROW LEVEL SECURITY;
ALTER TABLE public.team_members ENABLE ROW LEVEL SECURITY;

-- 删除三个表的全部旧 policy，避免 permissive policy 按 OR 继续放宽权限。
DO $$
DECLARE
    policy_record RECORD;
BEGIN
    FOR policy_record IN
        SELECT schemaname, tablename, policyname
        FROM pg_policies
        WHERE schemaname = 'public'
          AND tablename IN ('sync_data', 'teams', 'team_members')
    LOOP
        EXECUTE format(
            'DROP POLICY IF EXISTS %I ON %I.%I',
            policy_record.policyname,
            policy_record.schemaname,
            policy_record.tablename
        );
    END LOOP;
END;
$$;

-- sync_data：成员可读，owner/admin 可维护团队数据。
CREATE POLICY sync_data_select ON public.sync_data
FOR SELECT USING (
    (team_id IS NULL AND owner_id = auth.uid())
    OR (team_id IS NOT NULL AND team_id IN (SELECT public.get_my_team_ids()))
);

CREATE POLICY sync_data_insert ON public.sync_data
FOR INSERT WITH CHECK (
    owner_id = auth.uid()
    AND (team_id IS NULL OR public.can_edit_team_sync_data(team_id))
);

CREATE POLICY sync_data_update ON public.sync_data
FOR UPDATE USING (
    (team_id IS NULL AND owner_id = auth.uid())
    OR (team_id IS NOT NULL AND public.can_edit_team_sync_data(team_id))
) WITH CHECK (
    (team_id IS NULL AND owner_id = auth.uid())
    OR (team_id IS NOT NULL AND public.can_edit_team_sync_data(team_id))
);

CREATE POLICY sync_data_delete ON public.sync_data
FOR DELETE USING (
    (team_id IS NULL AND owner_id = auth.uid())
    OR (team_id IS NOT NULL AND public.can_edit_team_sync_data(team_id))
);

-- teams：成员可读，owner/admin 可修改团队，只有 owner 可以删除团队。
CREATE POLICY teams_select ON public.teams
FOR SELECT USING (id IN (SELECT public.get_my_team_ids()));

CREATE POLICY teams_insert ON public.teams
FOR INSERT WITH CHECK (owner_id = auth.uid());

CREATE POLICY teams_update ON public.teams
FOR UPDATE USING (public.can_manage_team_members(id))
WITH CHECK (public.can_manage_team_members(id));

CREATE POLICY teams_delete ON public.teams
FOR DELETE USING (owner_id = auth.uid());

-- team_members：admin 可管理 member/admin，但 owner 记录只能由 owner 管理。
CREATE POLICY team_members_select ON public.team_members
FOR SELECT USING (team_id IN (SELECT public.get_my_team_ids()));

CREATE POLICY team_members_insert ON public.team_members
FOR INSERT WITH CHECK (
    CASE
        WHEN role = 'owner' THEN public.can_manage_team_owners(team_id)
        ELSE public.can_manage_team_members(team_id)
    END
);

CREATE POLICY team_members_update ON public.team_members
FOR UPDATE USING (
    CASE
        WHEN role = 'owner' THEN public.can_manage_team_owners(team_id)
        ELSE public.can_manage_team_members(team_id)
    END
) WITH CHECK (
    CASE
        WHEN role = 'owner' THEN public.can_manage_team_owners(team_id)
        ELSE public.can_manage_team_members(team_id)
    END
);

CREATE POLICY team_members_delete ON public.team_members
FOR DELETE USING (
    (user_id = auth.uid() AND role <> 'owner')
    OR (
        user_id <> auth.uid()
        AND CASE
            WHEN role = 'owner' THEN public.can_manage_team_owners(team_id)
            ELSE public.can_manage_team_members(team_id)
        END
    )
);

COMMIT;
