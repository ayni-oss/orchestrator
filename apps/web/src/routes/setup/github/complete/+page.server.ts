import { error } from "@sveltejs/kit";
import type { PageServerLoad } from "./$types";

const stateCookie = "github_app_manifest_state";

type ManifestConversion = {
  id: number;
  client_id: string;
  client_secret: string;
  webhook_secret: string;
  pem: string;
  html_url: string;
  slug: string;
};

export const load: PageServerLoad = async ({ cookies, fetch, setHeaders, url }) => {
  setHeaders({ "cache-control": "no-store, max-age=0", "referrer-policy": "no-referrer" });
  const code = url.searchParams.get("code");
  const state = url.searchParams.get("state");
  const expectedState = cookies.get(stateCookie);
  cookies.delete(stateCookie, { path: "/setup/github" });
  if (!code || !state || !expectedState || state !== expectedState) {
    throw error(400, "GitHub App setup expired or could not be verified. Start again.");
  }

  const response = await fetch(`https://api.github.com/app-manifests/${encodeURIComponent(code)}/conversions`, {
    method: "POST",
    headers: { accept: "application/vnd.github+json", "user-agent": "ayni-orchestrator" },
  });
  if (!response.ok) {
    throw error(502, "GitHub could not create the App credentials. Start again.");
  }
  const converted = (await response.json()) as ManifestConversion;
  return {
    credentials: {
      appId: String(converted.id),
      clientId: converted.client_id,
      clientSecret: converted.client_secret,
      webhookSecret: converted.webhook_secret,
      privateKeyPem: converted.pem,
      appUrl: converted.html_url,
      slug: converted.slug,
    },
  };
};
