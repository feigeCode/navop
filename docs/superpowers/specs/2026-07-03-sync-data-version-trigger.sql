create or replace function public.bump_sync_data_version()
returns trigger
language plpgsql
as $$
begin
    if tg_op = 'UPDATE' then
        if old.encrypted_data is distinct from new.encrypted_data
           or old.key_version is distinct from new.key_version
           or old.checksum is distinct from new.checksum
           or old.deleted_at is distinct from new.deleted_at then
            new.version = old.version + 1;
            new.updated_at = now();
        end if;
    end if;
    return new;
end;
$$;

drop trigger if exists trg_bump_sync_data_version on public.sync_data;

create trigger trg_bump_sync_data_version
before update on public.sync_data
for each row
execute function public.bump_sync_data_version();

-- Deployment verification:
-- select tgname
-- from pg_trigger
-- where tgname = 'trg_bump_sync_data_version';
