import { createServerClient } from "@supabase/ssr";
import { cookies } from "next/headers";
import { getSupabaseEnv } from "@/lib/env";
import {
  readOneHubAuthSession,
  shouldUseOneHubAuthFallback
} from "@/lib/supabase/local-auth";

export async function createSupabaseServerClient() {
  const cookieStore = await cookies();
  const { url, anonKey } = getSupabaseEnv();

  const supabase = createServerClient(url, anonKey, {
    cookies: {
      getAll: () => cookieStore.getAll(),
      setAll: (cookiesToSet) => {
        try {
          cookiesToSet.forEach(({ name, value, options }) => {
            cookieStore.set(name, value, options);
          });
        } catch {
          // Server Components may be read-only; Server Actions and Route Handlers will persist cookies.
        }
      }
    }
  });

  const { data } = await supabase.auth.getSession();
  if (!data.session && shouldUseOneHubAuthFallback()) {
    const localSession = readOneHubAuthSession();
    if (localSession) {
      await supabase.auth.setSession({
        access_token: localSession.accessToken,
        refresh_token: localSession.refreshToken
      });
    }
  }

  return supabase;
}
