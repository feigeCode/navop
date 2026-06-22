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

Required environment variables:

```bash
NEXT_PUBLIC_SUPABASE_URL=
NEXT_PUBLIC_SUPABASE_ANON_KEY=
NEXT_PUBLIC_SITE_URL=http://localhost:3000
```

## Supabase Setup

Apply the migration in:

```text
supabase/migrations/202606220001_website_team_management.sql
```

The migration assumes the existing OnetCli Supabase project already has:

- `teams`
- `team_members`
- `sync_data`
- Supabase Auth enabled

It adds:

- `profiles`
- `team_invitations`
- `audit_events`
- `admin` support in `team_members.role`
- RPC helpers for creating teams, inviting members, updating roles, and removing members
- RLS policies for profiles, invitations, and audit events

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
