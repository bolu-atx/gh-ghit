// Glyphs drawn on a Canvas: nsh ships no icon set. 16-unit grid, stroked.
import { Canvas, token } from "nsh";
import type { CanvasRenderingContext2D, Tokens } from "nsh";
import type { Status } from "./graph";

export type Glyph =
  | "issue-open"
  | "issue-done"
  | "issue-shut"
  | "pr-open"
  | "pr-draft"
  | "pr-done"
  | "pr-shut"
  | "list-issues"
  | "list-prs"
  | "map"
  | "reload"
  | "repo"
  | "comment"
  | "fit";

type Cx = CanvasRenderingContext2D;

function ring(cx: Cx, x: number, y: number, r: number) {
  cx.beginPath();
  cx.arc(x, y, r, 0, Math.PI * 2);
  cx.stroke();
}

function dot(cx: Cx, x: number, y: number, r: number) {
  cx.beginPath();
  cx.arc(x, y, r, 0, Math.PI * 2);
  cx.fill();
}

function line(cx: Cx, pts: number[]) {
  cx.beginPath();
  cx.moveTo(pts[0], pts[1]);
  for (let i = 2; i < pts.length; i += 2) cx.lineTo(pts[i], pts[i + 1]);
  cx.stroke();
}

function prBase(cx: Cx) {
  ring(cx, 4, 3.5, 1.75);
  ring(cx, 4, 12.5, 1.75);
  line(cx, [4, 5.25, 4, 10.75]);
}

const DRAW: Record<Glyph, (cx: Cx) => void> = {
  "issue-open": (cx) => {
    ring(cx, 8, 8, 6.25);
    dot(cx, 8, 8, 1.6);
  },
  "issue-done": (cx) => {
    ring(cx, 8, 8, 6.25);
    line(cx, [5.4, 8.2, 7.2, 10, 10.8, 6.3]);
  },
  "issue-shut": (cx) => {
    ring(cx, 8, 8, 6.25);
    line(cx, [4, 12, 12, 4]);
  },
  "pr-open": (cx) => {
    prBase(cx);
    ring(cx, 12, 12.5, 1.75);
    line(cx, [12, 10.75, 12, 6.5, 11, 5, 8, 5]);
    line(cx, [9.6, 3.4, 8, 5, 9.6, 6.6]);
  },
  "pr-draft": (cx) => {
    prBase(cx);
    ring(cx, 12, 12.5, 1.75);
    dot(cx, 12, 8, 0.9);
    dot(cx, 12, 4.5, 0.9);
  },
  "pr-done": (cx) => {
    ring(cx, 4, 3.5, 1.75);
    ring(cx, 4, 12.5, 1.75);
    ring(cx, 12, 8.5, 1.75);
    line(cx, [4, 5.25, 4, 10.75]);
    cx.beginPath();
    cx.moveTo(4, 5.25);
    cx.quadraticCurveTo(4.5, 8.5, 10.25, 8.5);
    cx.stroke();
  },
  "pr-shut": (cx) => {
    prBase(cx);
    ring(cx, 12, 12.5, 1.75);
    line(cx, [10.3, 3.3, 13.7, 6.7]);
    line(cx, [13.7, 3.3, 10.3, 6.7]);
  },
  "list-issues": (cx) => {
    ring(cx, 8, 8, 6.25);
    dot(cx, 8, 8, 1.6);
  },
  "list-prs": (cx) => {
    prBase(cx);
    ring(cx, 12, 12.5, 1.75);
    line(cx, [12, 10.75, 12, 6.5, 11, 5, 8, 5]);
  },
  map: (cx) => {
    ring(cx, 3.5, 4, 1.9);
    ring(cx, 12.5, 3.5, 1.9);
    ring(cx, 8, 12.5, 1.9);
    line(cx, [5.3, 4, 10.6, 3.6]);
    line(cx, [4.4, 5.7, 7.1, 10.8]);
    line(cx, [11.6, 5.2, 8.9, 10.8]);
  },
  reload: (cx) => {
    cx.beginPath();
    cx.arc(8, 8, 5.5, -Math.PI * 0.35, Math.PI * 1.45);
    cx.stroke();
    line(cx, [10.2, 1.6, 12.6, 3.3, 10.6, 5.6]);
  },
  repo: (cx) => {
    cx.beginPath();
    // A book: cover, page edge, and a ribbon.
    cx.roundRect(3, 1.75, 10.5, 12.5, 1.5);
    cx.stroke();
    line(cx, [3, 11, 13.5, 11]);
    line(cx, [6.5, 11, 6.5, 15, 7.75, 14, 9, 15, 9, 11]);
  },
  comment: (cx) => {
    cx.beginPath();
    cx.roundRect(2, 2.5, 12, 8.5, 2);
    cx.stroke();
    line(cx, [5, 11, 5, 14, 8, 11]);
  },
  fit: (cx) => {
    line(cx, [2, 5.5, 2, 2, 5.5, 2]);
    line(cx, [10.5, 2, 14, 2, 14, 5.5]);
    line(cx, [14, 10.5, 14, 14, 10.5, 14]);
    line(cx, [5.5, 14, 2, 14, 2, 10.5]);
  },
};

/** A 16-unit glyph in a theme color, drawn at `size` points. */
export function Icon(props: { glyph: Glyph | (() => Glyph); color?: keyof Tokens | (() => keyof Tokens); size?: number }) {
  const size = props.size ?? 16;
  return (
    <Canvas
      style={{ width: size, height: size }}
      draw={(cx) => {
        const g = typeof props.glyph === "function" ? props.glyph() : props.glyph;
        const c = typeof props.color === "function" ? props.color() : (props.color ?? "foreground");
        const s = size / 16;
        cx.save();
        cx.scale(s, s);
        const ink = token(c) as string;
        cx.strokeStyle = ink;
        cx.fillStyle = ink;
        cx.lineWidth = 1.5;
        cx.lineCap = "round";
        cx.lineJoin = "round";
        DRAW[g](cx);
        cx.restore();
      }}
    />
  );
}

export function stateGlyph(kind: "issue" | "pr", status: Status): Glyph {
  if (kind === "issue") return status === "open" || status === "draft" ? "issue-open" : status === "done" ? "issue-done" : "issue-shut";
  return status === "draft" ? "pr-draft" : status === "open" ? "pr-open" : status === "done" ? "pr-done" : "pr-shut";
}

export function stateColor(status: Status): keyof Tokens {
  return status === "draft" ? "muted-foreground" : status;
}

/** Maps a `gh` item's state to a status: a closed PR is shut, a closed issue done unless not planned. */
export function rowStatus(kind: "issue" | "pr", state: string, isDraft?: boolean, stateReason?: string | null): Status {
  const s = state.toUpperCase();
  if (s === "OPEN") return isDraft ? "draft" : "open";
  if (s === "MERGED") return "done";
  if (kind === "pr") return "shut";
  return stateReason === "NOT_PLANNED" || stateReason === "DUPLICATE" ? "shut" : "done";
}
