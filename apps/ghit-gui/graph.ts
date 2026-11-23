// Dependency map: one GraphQL read through `gh`, then a small force simulation.
import { gql, splitRepo } from "./api";

/** How many recent issues and PRs the map reads (each). */
export const WINDOW = 100;

export type EdgeKind = "blocks" | "parent" | "closes";

export interface GNode {
  id: number;
  kind: "issue" | "pr";
  /** open, done (completed / merged), shut (not planned / closed PR), draft. */
  status: Status;
  title: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  fixed: boolean;
  deg: number;
}

export type Status = "open" | "done" | "shut" | "draft";

export interface GEdge {
  /** Indexes into `nodes`. */
  src: number;
  dst: number;
  kind: EdgeKind;
}

export interface Graph {
  nodes: GNode[];
  edges: GEdge[];
}

interface RawIssue {
  number: number;
  title: string;
  state: string;
  stateReason: string | null;
  parent: { number: number } | null;
  blockedBy: { nodes: ({ number: number } | null)[] } | null;
}

interface RawPr {
  number: number;
  title: string;
  state: string;
  isDraft: boolean;
  closingIssuesReferences: { nodes: ({ number: number } | null)[] } | null;
}

export interface RawGraph {
  issues: RawIssue[];
  prs: RawPr[];
}

const QUERY = `query($owner: String!, $name: String!) {
  repository(owner: $owner, name: $name) {
    issues(last: ${WINDOW}) { nodes {
      number title state stateReason
      parent { number }
      blockedBy(first: 10) { nodes { number } }
    } }
    pullRequests(last: ${WINDOW}) { nodes {
      number title state isDraft
      closingIssuesReferences(first: 5) { nodes { number } }
    } }
  }
}`;

export async function fetchGraph(repo: string): Promise<RawGraph> {
  const r = (await gql<{ repository: { issues: { nodes: RawIssue[] }; pullRequests: { nodes: RawPr[] } } }>(QUERY, splitRepo(repo)))
    .repository;
  return { issues: r.issues.nodes, prs: r.pullRequests.nodes };
}

function issueStatus(i: RawIssue): Status {
  if (i.state === "OPEN") return "open";
  return i.stateReason === "NOT_PLANNED" || i.stateReason === "DUPLICATE" ? "shut" : "done";
}

function prStatus(p: RawPr): Status {
  if (p.state === "MERGED") return "done";
  if (p.state === "CLOSED") return "shut";
  return p.isDraft ? "draft" : "open";
}

/** Builds nodes and edges; links to items outside the window are dropped. */
export function buildGraph(raw: RawGraph): Graph {
  const nodes: GNode[] = [];
  const index = new Map<number, number>();
  const add = (id: number, kind: GNode["kind"], status: Status, title: string) => {
    // Phyllotaxis seed: deterministic and evenly spread.
    const i = nodes.length;
    const r = 14 * Math.sqrt(i + 1);
    const a = i * 2.39996;
    index.set(id, i);
    nodes.push({ id, kind, status, title, x: r * Math.cos(a), y: r * Math.sin(a), vx: 0, vy: 0, fixed: false, deg: 0 });
  };
  for (const i of raw.issues) add(i.number, "issue", issueStatus(i), i.title);
  for (const p of raw.prs) add(p.number, "pr", prStatus(p), p.title);

  const edges: GEdge[] = [];
  const seen = new Set<string>();
  const link = (from: number, to: number, kind: EdgeKind) => {
    const s = index.get(from);
    const d = index.get(to);
    if (s === undefined || d === undefined || s === d) return;
    const key = `${s}:${d}:${kind}`;
    if (seen.has(key)) return;
    seen.add(key);
    edges.push({ src: s, dst: d, kind });
    nodes[s].deg++;
    nodes[d].deg++;
  };
  for (const i of raw.issues) {
    if (i.parent) link(i.parent.number, i.number, "parent");
    for (const b of i.blockedBy?.nodes ?? []) if (b) link(b.number, i.number, "blocks");
  }
  for (const p of raw.prs) {
    for (const c of p.closingIssuesReferences?.nodes ?? []) if (c) link(p.number, c.number, "closes");
  }
  return { nodes, edges };
}

// ---- simulation (d3-force style, tuned for a few hundred nodes) ----

const LINK_LEN: Record<EdgeKind, number> = { parent: 46, blocks: 64, closes: 40 };
const CHARGE = -90;
const CHARGE_CUTOFF2 = 420 * 420;
const GRAVITY = 0.035;
const DECAY = 0.6;
export const ALPHA_MIN = 0.004;

/** One step over the nodes `on` marks; returns the next alpha. */
export function tick(g: Graph, on: boolean[], alpha: number): number {
  const n = g.nodes;
  for (let i = 0; i < n.length; i++) {
    if (!on[i]) continue;
    const a = n[i];
    for (let j = i + 1; j < n.length; j++) {
      if (!on[j]) continue;
      const b = n[j];
      let dx = b.x - a.x;
      let dy = b.y - a.y;
      let d2 = dx * dx + dy * dy;
      if (d2 > CHARGE_CUTOFF2) continue;
      if (d2 < 1) {
        dx = (i - j) * 0.01 + 0.1;
        dy = 0.1;
        d2 = 1;
      }
      const f = (CHARGE * alpha) / d2;
      a.vx += dx * f;
      a.vy += dy * f;
      b.vx -= dx * f;
      b.vy -= dy * f;
    }
  }
  for (const e of g.edges) {
    if (!on[e.src] || !on[e.dst]) continue;
    const s = n[e.src];
    const t = n[e.dst];
    const dx = t.x + t.vx - s.x - s.vx;
    const dy = t.y + t.vy - s.y - s.vy;
    const l = Math.sqrt(dx * dx + dy * dy) || 1;
    const k = ((l - LINK_LEN[e.kind]) / l) * alpha * 0.45;
    t.vx -= dx * k;
    t.vy -= dy * k;
    s.vx += dx * k;
    s.vy += dy * k;
  }
  for (let i = 0; i < n.length; i++) {
    const p = n[i];
    if (!on[i]) continue;
    if (p.fixed) {
      p.vx = 0;
      p.vy = 0;
      continue;
    }
    p.vx -= p.x * GRAVITY * alpha;
    p.vy -= p.y * GRAVITY * alpha;
    p.vx *= DECAY;
    p.vy *= DECAY;
    p.x += p.vx;
    p.y += p.vy;
  }
  return alpha + (0 - alpha) * 0.02;
}

/** The node's drawn radius in world units. */
export function radius(p: GNode): number {
  return 4.5 + Math.min(6, Math.sqrt(p.deg) * 2);
}

/** Bounds of the shown nodes, or null when none show. */
export function bounds(g: Graph, on: boolean[]): { x0: number; y0: number; x1: number; y1: number } | null {
  let x0 = Infinity;
  let y0 = Infinity;
  let x1 = -Infinity;
  let y1 = -Infinity;
  g.nodes.forEach((p, i) => {
    if (!on[i]) return;
    x0 = Math.min(x0, p.x);
    y0 = Math.min(y0, p.y);
    x1 = Math.max(x1, p.x);
    y1 = Math.max(y1, p.y);
  });
  return x0 === Infinity ? null : { x0, y0, x1, y1 };
}
