# OnetCli Website

This directory is intentionally self-contained so it can be copied into a new repository later.

## Stack

- Next.js App Router
- Supabase Auth, Postgres, RLS, RPC
- React Server Components and Server Actions
- Built-in i18n routes: `zh-CN`, `en-US`, `zh-TW`
- CSS variables for light/dark theme tokens

## Local Setup

```bash
npm install
cp .env.example .env.local
npm run dev
```

Local Supabase is the default. Start it before opening the dashboard:

```bash
supabase start
supabase db push
```

Environment variables are only needed when you want to override the local stack:

```bash
NEXT_PUBLIC_SUPABASE_URL=http://127.0.0.1:54321
NEXT_PUBLIC_SUPABASE_ANON_KEY=<local-or-remote-anon-key>
NEXT_PUBLIC_SITE_URL=http://localhost:3000
ONETCLI_ONE_HUB_AUTH_PATH=
```

Formal login uses Supabase Auth in the website. During local development only, the server client can also read the desktop app session from:

```text
~/Library/Application Support/one-hub/auth.json
```

That shortcut is only for checking the dashboard/team-management flow against a local Supabase session. It is ignored when `NODE_ENV=production`, and you can disable it locally with `ONETCLI_ENABLE_ONE_HUB_AUTH=0`. Set `ONETCLI_ONE_HUB_AUTH_PATH` to point at a different auth file. The token file is read only on the server side and must never be committed.

## Supabase Setup

The local Supabase config lives in:

```text
supabase/config.toml
```

Apply the migration in:

```text
supabase/migrations/202606220001_website_team_management.sql
```

The migration creates or updates:

- `teams`
- `team_members`
- `profiles`
- `team_invitations`
- `audit_events`
- `admin` support in `team_members.role`
- RPC helpers for creating teams, inviting members, updating roles, and removing members
- RLS policies for profiles, invitations, and audit events

It assumes Supabase Auth is enabled. The desktop sync table `sync_data` remains owned by the desktop sync schema.

## Migration Out Of This Worktree

To move this website into a standalone repository:

1. Copy the contents of `website/` to the new repository root.
2. Keep `public/brand` and `public/screenshots` with the project.
3. Set the Supabase environment variables in the hosting platform.
4. Apply the Supabase migration to the shared OnetCli project.
5. Run `npm install`, `npm run typecheck`, `npm run test`, and `npm run build`.

## Desktop Compatibility

The website owns all team management flows: creating teams, inviting members, updating roles, and removing members.

The desktop client only reads teams and team members from Supabase for sync, team dropdowns, and the local permission cache. It understands `owner`, `admin`, and `member`; `owner` and `admin` can edit team connections locally.
