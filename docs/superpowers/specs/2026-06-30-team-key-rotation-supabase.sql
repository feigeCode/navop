-- Deploy this function to Supabase before enabling client-side team key rotation.
-- It updates a team's key verification metadata and rewrites all supplied sync_data
-- records in a single transaction. RLS still applies to the caller.

create or replace function public.rotate_team_key(
    p_team_id uuid,
    p_key_verification text,
    p_key_version integer,
    p_records jsonb
)
returns void
language plpgsql
security invoker
as $$
declare
    v_record jsonb;
    v_updated integer;
begin
    if p_key_verification is null or length(p_key_verification) = 0 then
        raise exception 'key verification is required';
    end if;

    update public.teams
       set key_verification = p_key_verification,
           key_version = p_key_version,
           updated_at = now()
     where id = p_team_id;

    if not found then
        raise exception 'team not found or no permission';
    end if;

    for v_record in select * from jsonb_array_elements(coalesce(p_records, '[]'::jsonb))
    loop
        update public.sync_data
           set encrypted_data = v_record->>'encrypted_data',
               key_version = (v_record->>'key_version')::integer,
               checksum = v_record->>'checksum',
               deleted_at = case
                   when v_record ? 'deleted_at' and v_record->>'deleted_at' is not null
                   then (v_record->>'deleted_at')::timestamptz
                   else deleted_at
               end,
               updated_at = now(),
               version = version + 1
         where id = (v_record->>'id')::uuid
           and team_id = p_team_id
           and version = (v_record->>'version')::integer;

        get diagnostics v_updated = row_count;
        if v_updated <> 1 then
            raise exception 'sync_data version conflict: %', v_record->>'id';
        end if;
    end loop;
end;
$$;
