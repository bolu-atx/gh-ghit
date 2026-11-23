// GitHub access for every platform: token-backed GraphQL and REST over fetch.
// The token comes from `gh auth token` on desktop, and from the user on the web
// (browsers cannot spawn processes), kept in the app's storage.
import { platform, Storage } from "nsh";
import process from "process";

const API = "https://api.github.com";
const TOKEN_KEY = "github-token";

export const isWeb = platform.name === "web";

let token: string | null = null;

export class AuthError extends Error {}

/** Finds a token: the stored one on the web, gh's on desktop. Null when there is none. */
export async function loadToken(): Promise<string | null> {
  if (token) return token;
  if (isWeb) {
    token = await Storage.getItem(TOKEN_KEY);
    return token;
  }
  const out = await process.run("gh", ["auth", "token"], { timeoutMs: 10_000 });
  token = out.code === 0 ? out.stdout.trim() || null : null;
  return token;
}

/** Checks a pasted token and keeps it; resolves with the account's login. */
export async function saveToken(candidate: string): Promise<string> {
  const t = candidate.trim();
  const login = (await gql<{ viewer: { login: string } }>("query { viewer { login } }", {}, t)).viewer.login;
  token = t;
  if (isWeb) await Storage.setItem(TOKEN_KEY, t);
  return login;
}

export async function forgetToken(): Promise<void> {
  token = null;
  if (isWeb) await Storage.removeItem(TOKEN_KEY);
}

function need(t: string | null): string {
  if (!t) throw new AuthError(isWeb ? "Connect a GitHub token to continue." : "gh has no token. Run gh auth login.");
  return t;
}

async function check(res: Response): Promise<void> {
  if (res.status === 401) {
    await forgetToken();
    throw new AuthError("GitHub refused the token. Connect a new one.");
  }
  if (!res.ok) throw new Error(`GitHub answered ${res.status} ${res.statusText}`);
}

export async function gql<T>(query: string, variables: Record<string, unknown>, override?: string): Promise<T> {
  const t = need(override ?? token);
  const res = await fetch(`${API}/graphql`, {
    method: "POST",
    headers: { Authorization: `Bearer ${t}`, "Content-Type": "application/json" },
    body: JSON.stringify({ query, variables }),
  });
  await check(res);
  const body = (await res.json()) as { data?: T; errors?: { message: string }[] };
  if (body.errors?.length) throw new Error(body.errors.map((e) => e.message).join("; "));
  return body.data as T;
}

/** A REST GET as text (`accept` picks the media type, e.g. a PR's diff). */
export async function restText(path: string, accept: string): Promise<string> {
  const res = await fetch(`${API}${path}`, { headers: { Authorization: `Bearer ${need(token)}`, Accept: accept } });
  await check(res);
  return res.text();
}

export function splitRepo(repo: string): { owner: string; name: string } {
  const [owner, name] = repo.split("/");
  if (!owner || !name) throw new Error(`"${repo}" is not owner/name`);
  return { owner, name };
}
