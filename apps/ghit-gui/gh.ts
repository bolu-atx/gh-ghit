// Read-only data layer over the GitHub API (see api.ts for the token).
import { gql, restText, splitRepo } from "./api";

export type Kind = "issue" | "pr";

export interface Label {
  name: string;
  color: string;
}

export interface Row {
  number: number;
  title: string;
  state: string;
  author: { login: string } | null;
  labels: Label[];
  updatedAt: string;
  isDraft?: boolean;
  stateReason?: string | null;
}

export interface Comment {
  author: { login: string } | null;
  body: string;
  createdAt: string;
}

export interface Detail extends Row {
  body: string;
  createdAt: string;
  url: string;
  comments: Comment[];
  headRefName?: string;
  baseRefName?: string;
  additions?: number;
  deletions?: number;
}

const ROW = "number title state updatedAt author { login } labels(first: 10) { nodes { name color } }";

const LIST = {
  issue: `query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) {
    items: issues(first: 100, states: OPEN, orderBy: { field: UPDATED_AT, direction: DESC }) { nodes { ${ROW} stateReason } }
  } }`,
  pr: `query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) {
    items: pullRequests(first: 100, states: OPEN, orderBy: { field: UPDATED_AT, direction: DESC }) { nodes { ${ROW} isDraft } }
  } }`,
};

const COMMON = `${ROW} body createdAt url comments(first: 100) { nodes { author { login } body createdAt } }`;

const DETAIL = `query($owner: String!, $name: String!, $n: Int!) { repository(owner: $owner, name: $name) {
  issueOrPullRequest(number: $n) {
    ... on Issue { ${COMMON} stateReason }
    ... on PullRequest { ${COMMON} isDraft headRefName baseRefName additions deletions }
  }
} }`;

type Raw<T> = Omit<T, "labels" | "comments"> & { labels: { nodes: Label[] }; comments?: { nodes: Comment[] } };

export async function listRows(repo: string, kind: Kind): Promise<Row[]> {
  const d = await gql<{ repository: { items: { nodes: Raw<Row>[] } } }>(LIST[kind], splitRepo(repo));
  return d.repository.items.nodes.map((n) => ({ ...n, labels: n.labels.nodes }));
}

export async function viewDetail(repo: string, _kind: Kind, n: number): Promise<Detail> {
  const d = await gql<{ repository: { issueOrPullRequest: Raw<Detail> | null } }>(DETAIL, { ...splitRepo(repo), n });
  const item = d.repository.issueOrPullRequest;
  if (!item) throw new Error(`#${n} is not in ${repo}`);
  return { ...item, labels: item.labels.nodes, comments: item.comments?.nodes ?? [] };
}

export async function prDiff(repo: string, n: number): Promise<string> {
  const { owner, name } = splitRepo(repo);
  return restText(`/repos/${owner}/${name}/pulls/${n}`, "application/vnd.github.diff");
}

// ---- pure helpers (unit tested) ----

export type DiffKind = "file" | "hunk" | "add" | "del" | "ctx";

export interface DiffLine {
  id: number;
  kind: DiffKind;
  text: string;
  /** Line numbers in the old and new file; absent on file and hunk rows. */
  old?: number;
  new?: number;
}

const HUNK = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@ ?(.*)$/;

/** Parses `gh pr diff` output: file rows show the path, git metadata is dropped,
 * and each code line carries its old and new line numbers. */
export function parseDiff(raw: string): DiffLine[] {
  const out: DiffLine[] = [];
  let o = 0;
  let n = 0;
  let inHeader = false;
  for (const line of raw.split("\n")) {
    if (line.startsWith("diff --git ")) {
      const path = line.replace(/^diff --git a\/.* b\//, "");
      out.push({ id: out.length, kind: "file", text: path });
      inHeader = true;
      continue;
    }
    const h = HUNK.exec(line);
    if (h) {
      o = Number(h[1]);
      n = Number(h[2]);
      out.push({ id: out.length, kind: "hunk", text: h[3] ?? "" });
      inHeader = false;
      continue;
    }
    if (inHeader) {
      if (line.startsWith("Binary files")) out.push({ id: out.length, kind: "ctx", text: "Binary file changed" });
      continue;
    }
    if (line.startsWith("\\")) continue;
    if (line.startsWith("+")) out.push({ id: out.length, kind: "add", text: line.slice(1), new: n++ });
    else if (line.startsWith("-")) out.push({ id: out.length, kind: "del", text: line.slice(1), old: o++ });
    else if (line.startsWith(" ")) out.push({ id: out.length, kind: "ctx", text: line.slice(1), old: o++, new: n++ });
  }
  return out;
}

export function relTime(iso: string, now: number = Date.now()): string {
  const s = Math.max(0, Math.floor((now - Date.parse(iso)) / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  if (s < 86400 * 30) return `${Math.floor(s / 86400)}d`;
  if (s < 86400 * 365) return `${Math.floor(s / (86400 * 30))}mo`;
  return `${Math.floor(s / (86400 * 365))}y`;
}

/** Moves a selection by `delta`, clamped to `[0, len)`; -1 when empty. */
export function step(sel: number, delta: number, len: number): number {
  if (len === 0) return -1;
  return Math.min(len - 1, Math.max(0, sel + delta));
}
