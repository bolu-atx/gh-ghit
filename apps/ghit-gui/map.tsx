// The dependency map: a Canvas, a force simulation, and native chrome on top.
import { Canvas, cleanup, effect, keymap, signal, Text, token, untracked, View } from "nsh";
import type { JSX } from "nsh/jsx-runtime";
import type { CanvasRenderingContext2D, Tokens } from "nsh";
import type { Target } from "./detail";
import { Message } from "./detail";
import { ALPHA_MIN, WINDOW, bounds, buildGraph, fetchGraph, type GEdge, type GNode, type Graph, radius, tick } from "./graph";
import { Icon, stateColor } from "./icons";

interface Cam {
  x: number;
  y: number;
  k: number;
}

const K_MIN = 0.25;
const K_MAX = 4;
const LABEL_ZOOM = 1.5;
const WARM_TICKS = 140;

const clampK = (k: number) => Math.min(K_MAX, Math.max(K_MIN, k));

export function MapView(props: { repo: () => string; selected: () => Target | null; onSelect: (t: Target | null) => void }) {
  const graph = signal<Graph | null>(null);
  const error = signal<string | null>(null);
  const showAll = signal(false);
  const frame = signal(0);
  const hover = signal(-1);
  const cam = signal<Cam>({ x: 0, y: 0, k: 1 });

  let on: boolean[] = [];
  let alpha = 0;
  let running = false;
  let disposed = false;
  let w = 0;
  let h = 0;
  let fitPending = true;
  // Pointer drag: a node, or the camera.
  let drag: { node: number; sx: number; sy: number; cam: Cam; moved: boolean } | null = null;
  let seq = 0;

  cleanup(() => (disposed = true));

  const selectedIndex = () => {
    const g = graph();
    const t = props.selected();
    if (!g || !t) return -1;
    return g.nodes.findIndex((n) => n.id === t.number);
  };

  function recomputeOn() {
    const g = graph();
    if (!g) return;
    const all = showAll();
    const sel = untracked(selectedIndex);
    on = g.nodes.map((n, i) => all || n.deg > 0 || i === sel);
  }

  function loop() {
    const g = graph();
    if (disposed || !g) {
      running = false;
      return;
    }
    alpha = tick(g, on, alpha);
    frame.update((f) => f + 1);
    if (alpha > ALPHA_MIN || drag) requestAnimationFrame(loop);
    else running = false;
  }

  function kick(a: number) {
    alpha = Math.max(alpha, a);
    if (!running) {
      running = true;
      requestAnimationFrame(loop);
    }
  }

  async function load(repo: string) {
    const my = ++seq;
    graph.set(null);
    error.set(null);
    try {
      const g = buildGraph(await fetchGraph(repo));
      if (my !== seq) return;
      graph.set(g);
      recomputeOn();
      // Settle most of the layout before the first frame; the rest animates.
      let a = 1;
      for (let i = 0; i < WARM_TICKS; i++) a = tick(g, on, a);
      alpha = a;
      fitPending = true;
      kick(a);
    } catch (e) {
      if (my === seq) error.set(String((e as Error).message ?? e));
    }
  }

  effect(() => {
    const r = props.repo();
    untracked(() => void load(r));
  });

  effect(() => {
    showAll();
    untracked(() => {
      recomputeOn();
      fitPending = true;
      kick(0.5);
    });
  });

  // ---- camera ----

  function toWorld(x: number, y: number) {
    const c = cam();
    return { x: (x - w / 2) / c.k + c.x, y: (y - h / 2) / c.k + c.y };
  }

  function toScreen(p: { x: number; y: number }, c: Cam) {
    return { x: (p.x - c.x) * c.k + w / 2, y: (p.y - c.y) * c.k + h / 2 };
  }

  function fit() {
    const g = graph();
    if (!g || !w) return;
    const b = bounds(g, on);
    if (!b) return;
    const pad = 90;
    const k = clampK(Math.min(w / (b.x1 - b.x0 + pad * 2), h / (b.y1 - b.y0 + pad * 2)));
    cam.set({ x: (b.x0 + b.x1) / 2, y: (b.y0 + b.y1) / 2, k: Math.min(k, 2) });
  }

  function zoomAt(x: number, y: number, factor: number) {
    const c = cam();
    const k = clampK(c.k * factor);
    const before = toWorld(x, y);
    // Keep the world point under the pointer fixed.
    cam.set({ x: before.x - (x - w / 2) / k, y: before.y - (y - h / 2) / k, k });
  }

  function centerOn(i: number) {
    const g = graph();
    if (!g) return;
    const c = cam();
    cam.set({ ...c, x: g.nodes[i].x, y: g.nodes[i].y });
  }

  function hit(x: number, y: number): number {
    const g = graph();
    if (!g) return -1;
    const c = cam();
    let best = -1;
    let bestD = Infinity;
    g.nodes.forEach((n, i) => {
      if (!on[i]) return;
      const s = toScreen(n, c);
      const d = Math.hypot(s.x - x, s.y - y);
      if (d < radius(n) * c.k + 5 && d < bestD) {
        best = i;
        bestD = d;
      }
    });
    return best;
  }

  function select(i: number) {
    const g = graph();
    if (!g || i < 0) return props.onSelect(null);
    const n = g.nodes[i];
    props.onSelect({ kind: n.kind, number: n.id });
  }

  /** Moves the selection through shown nodes in number order. */
  function stepSelection(delta: number) {
    const g = graph();
    if (!g) return;
    const shown = g.nodes.map((n, i) => ({ n, i })).filter(({ i }) => on[i]).sort((a, b) => a.n.id - b.n.id);
    if (!shown.length) return;
    const cur = shown.findIndex(({ i }) => i === selectedIndex());
    const next = shown[Math.min(shown.length - 1, Math.max(0, cur < 0 ? 0 : cur + delta))].i;
    select(next);
    centerOn(next);
  }

  keymap.bind(
    {
      f: () => fit(),
      u: () => showAll.update((v) => !v),
      j: () => stepSelection(1),
      k: () => stepSelection(-1),
      escape: () => props.onSelect(null),
      "=": () => zoomAt(w / 2, h / 2, 1.25),
      "-": () => zoomAt(w / 2, h / 2, 0.8),
    },
    { when: "notEditing" },
  );

  // ---- drawing ----

  function draw(cx: CanvasRenderingContext2D, size: { width: number; height: number }) {
    frame();
    const g = graph();
    const c = cam();
    const hov = hover();
    const sel = selectedIndex();
    w = size.width;
    h = size.height;
    if (!g) return;
    if (fitPending && w > 0) {
      fitPending = false;
      untracked(fit);
      return;
    }
    const ink = (t: keyof Tokens) => token(t) as string;
    const focus = hov >= 0 ? hov : sel;
    const near = new Set<number>();
    if (focus >= 0) {
      near.add(focus);
      for (const e of g.edges) {
        if (e.src === focus) near.add(e.dst);
        if (e.dst === focus) near.add(e.src);
      }
    }
    const dimmed = (i: number) => focus >= 0 && !near.has(i);

    for (const e of g.edges) {
      if (!on[e.src] || !on[e.dst]) continue;
      const lit = focus >= 0 && (e.src === focus || e.dst === focus);
      drawEdge(cx, g, e, c, lit ? 0.95 : focus >= 0 ? 0.1 : 0.55, ink);
    }

    g.nodes.forEach((n, i) => {
      if (!on[i]) return;
      drawNode(cx, n, toScreen(n, c), radius(n) * c.k, dimmed(i) ? 0.22 : 1, i === sel, ink);
    });

    cx.font = "500 11px sans-serif";
    cx.textAlign = "center";
    cx.textBaseline = "top";
    cx.fontVariantNumeric = "tabular-nums";
    g.nodes.forEach((n, i) => {
      if (!on[i]) return;
      if (!(near.has(i) || (c.k >= LABEL_ZOOM && focus < 0))) return;
      const s = toScreen(n, c);
      cx.globalAlpha = 1;
      cx.fillStyle = ink(i === focus ? "foreground" : "muted-foreground");
      cx.fillText(`#${n.id}`, s.x, s.y + radius(n) * c.k + 4);
    });

    if (hov >= 0) drawTip(cx, g.nodes[hov], toScreen(g.nodes[hov], c), radius(g.nodes[hov]) * c.k, ink);
    cx.globalAlpha = 1;
  }

  function drawEdge(cx: CanvasRenderingContext2D, g: Graph, e: GEdge, c: Cam, a: number, ink: (t: keyof Tokens) => string) {
    const s = toScreen(g.nodes[e.src], c);
    const t = toScreen(g.nodes[e.dst], c);
    const dx = t.x - s.x;
    const dy = t.y - s.y;
    const len = Math.hypot(dx, dy) || 1;
    const ux = dx / len;
    const uy = dy / len;
    const rs = radius(g.nodes[e.src]) * c.k + 1.5;
    const rt = radius(g.nodes[e.dst]) * c.k + 2.5;
    if (len < rs + rt) return;
    const x0 = s.x + ux * rs;
    const y0 = s.y + uy * rs;
    const x1 = t.x - ux * rt;
    const y1 = t.y - uy * rt;
    const color = ink(e.kind === "blocks" ? "block" : e.kind === "closes" ? "primary" : "muted-foreground");
    cx.globalAlpha = a;
    cx.strokeStyle = color;
    cx.fillStyle = color;
    cx.lineWidth = e.kind === "blocks" ? 1.6 : 1.2;
    cx.setLineDash(e.kind === "closes" ? [4, 3] : []);
    cx.beginPath();
    cx.moveTo(x0, y0);
    cx.lineTo(x1, y1);
    cx.stroke();
    cx.setLineDash([]);
    // Arrowhead at the target: blocks → blocked, parent → child, PR → issue.
    const ah = Math.min(7, 3 + 3 * c.k);
    cx.beginPath();
    cx.moveTo(x1, y1);
    cx.lineTo(x1 - ux * ah - uy * ah * 0.55, y1 - uy * ah + ux * ah * 0.55);
    cx.lineTo(x1 - ux * ah + uy * ah * 0.55, y1 - uy * ah - ux * ah * 0.55);
    cx.closePath();
    cx.fill();
  }

  function drawNode(
    cx: CanvasRenderingContext2D,
    n: GNode,
    s: { x: number; y: number },
    r: number,
    a: number,
    selected: boolean,
    ink: (t: keyof Tokens) => string,
  ) {
    const color = ink(stateColor(n.status));
    const open = n.status === "open" || n.status === "draft";
    cx.globalAlpha = a;
    const shape = (rr: number) => {
      cx.beginPath();
      if (n.kind === "issue") cx.arc(s.x, s.y, rr, 0, Math.PI * 2);
      else cx.roundRect(s.x - rr, s.y - rr, rr * 2, rr * 2, rr * 0.45);
    };
    // Open items are solid; closed and merged ones are rings on the ground.
    shape(r);
    cx.fillStyle = open ? color : ink("background");
    cx.fill();
    cx.strokeStyle = color;
    cx.lineWidth = open ? 1 : 1.75;
    cx.stroke();
    if (selected) {
      shape(r + 4);
      cx.strokeStyle = ink("ring");
      cx.lineWidth = 2;
      cx.stroke();
    }
  }

  function drawTip(cx: CanvasRenderingContext2D, n: GNode, s: { x: number; y: number }, r: number, ink: (t: keyof Tokens) => string) {
    const title = n.title.length > 64 ? `${n.title.slice(0, 63)}…` : n.title;
    cx.globalAlpha = 1;
    cx.font = "500 12px sans-serif";
    const num = `#${n.id}  `;
    const wNum = cx.measureText(num).width;
    const wTitle = cx.measureText(title).width;
    const pw = wNum + wTitle + 20;
    const ph = 28;
    let x = s.x - pw / 2;
    let y = s.y - r - ph - 10;
    x = Math.max(8, Math.min(w - pw - 8, x));
    if (y < 8) y = s.y + r + 22;
    cx.shadowColor = "rgba(0,0,0,0.25)";
    cx.shadowBlur = 12;
    cx.beginPath();
    cx.roundRect(x, y, pw, ph, 6);
    cx.fillStyle = ink("popover");
    cx.fill();
    cx.shadowBlur = 0;
    cx.strokeStyle = ink("border");
    cx.lineWidth = 1;
    cx.stroke();
    cx.textAlign = "left";
    cx.textBaseline = "middle";
    cx.fillStyle = ink("muted-foreground");
    cx.fillText(num, x + 10, y + ph / 2);
    cx.fillStyle = ink("popover-foreground");
    cx.fillText(title, x + 10 + wNum, y + ph / 2);
  }

  // ---- counts for the chrome ----

  const counts = () => {
    frame();
    const g = graph();
    if (!g) return { shown: 0, total: 0, links: 0 };
    return { shown: on.filter(Boolean).length, total: g.nodes.length, links: g.edges.length };
  };

  return (
    <View class="flex-1 bg-background">
      <Canvas
        class="flex-1"
        draw={draw}
        onStartShouldSetResponder={() => true}
        onResponderGrant={(e) => {
          const i = hit(e.locationX, e.locationY);
          drag = { node: i, sx: e.locationX, sy: e.locationY, cam: cam(), moved: false };
          if (i >= 0) graph()!.nodes[i].fixed = true;
        }}
        onResponderMove={(e) => {
          if (!drag) return;
          const dx = e.locationX - drag.sx;
          const dy = e.locationY - drag.sy;
          if (Math.hypot(dx, dy) > 3) drag.moved = true;
          if (!drag.moved) return;
          if (drag.node >= 0) {
            const p = toWorld(e.locationX, e.locationY);
            const n = graph()!.nodes[drag.node];
            n.x = p.x;
            n.y = p.y;
            kick(0.25);
          } else {
            cam.set({ ...drag.cam, x: drag.cam.x - dx / drag.cam.k, y: drag.cam.y - dy / drag.cam.k });
          }
        }}
        onResponderRelease={() => {
          if (!drag) return;
          const d = drag;
          drag = null;
          if (d.node >= 0) graph()!.nodes[d.node].fixed = false;
          if (!d.moved) select(d.node);
          else if (d.node >= 0) kick(0.15);
        }}
        onWheel={(e) => {
          const zoom = e.deltaMode === "lines" || e.modifiers.platform || e.modifiers.control;
          if (zoom) {
            const step = e.deltaMode === "lines" ? 0.12 : 0.01;
            zoomAt(e.position.x, e.position.y, Math.exp(-e.delta.y * step));
          } else {
            const c = cam();
            cam.set({ ...c, x: c.x + e.delta.x / c.k, y: c.y + e.delta.y / c.k });
          }
        }}
        onPinch={(e) => zoomAt(e.position.x, e.position.y, 1 + e.delta)}
        onMouseMove={(e) => {
          if (drag) return;
          const i = hit(e.position.x, e.position.y);
          if (i !== hover()) hover.set(i);
        }}
        onMouseLeave={() => hover.set(-1)}
      />

      {error() ? (
        <View class="absolute inset-0 bg-background">
          <Message title="The map did not load" body={`${error()}\nCheck the repository name, or run gh auth login.`} />
        </View>
      ) : !graph() ? (
        <View class="absolute inset-0">
          <Message title="Reading links" body={`Fetching the last ${WINDOW} issues and pull requests of ${props.repo()}.`} />
        </View>
      ) : null}

      <View class="absolute left-4 top-4 gap-2 rounded-(--radius) border border-border bg-popover/95 px-3 py-2.5">
        <LegendRow swatch={<Shape kind="issue" color="open" />} label="Open issue" />
        <LegendRow swatch={<Shape kind="pr" color="open" />} label="Open pull request" />
        <LegendRow swatch={<Shape kind="issue" color="done" hollow />} label="Closed or merged" />
        <View class="h-px bg-border my-0.5" />
        <LegendRow swatch={<Edge color="block" />} label="Blocks" />
        <LegendRow swatch={<Edge color="muted-foreground" />} label="Parent of" />
        <LegendRow swatch={<Edge color="primary" dashed />} label="Closes" />
      </View>

      <View class="absolute right-4 top-4 flex-row items-center gap-2">
        <Text class="text-[12px] text-muted-foreground pr-1">
          {graph() ? `${counts().shown} of ${counts().total} items, ${counts().links} links` : ""}
        </Text>
        <ChromeButton onPress={() => showAll.update((v) => !v)} active={showAll()}>
          <Text class="text-[12px] text-foreground">{showAll() ? "Hide unlinked" : "Show unlinked"}</Text>
        </ChromeButton>
        <ChromeButton onPress={fit}>
          <Icon glyph="fit" color="foreground" size={13} />
          <Text class="text-[12px] text-foreground">Fit</Text>
        </ChromeButton>
      </View>

      <Text class="absolute left-4 bottom-3 text-[11px] text-muted-foreground">
        Drag to pan or move a node. Pinch, or scroll with ⌘, to zoom.
      </Text>
    </View>
  );
}

function ChromeButton(props: { onPress: () => void; active?: boolean; children?: JSX.Element }) {
  return (
    <View
      role="button"
      onPress={props.onPress}
      class={`flex-row items-center gap-1.5 rounded-(--radius) border px-2.5 py-1 ${props.active ? "border-primary bg-accent" : "border-border bg-popover"} hover:bg-accent`}
    >
      {props.children}
    </View>
  );
}

function LegendRow(props: { swatch: JSX.Element; label: string }) {
  return (
    <View class="flex-row items-center gap-2.5">
      <View class="w-5 items-center">{props.swatch}</View>
      <Text class="text-[12px] text-muted-foreground">{props.label}</Text>
    </View>
  );
}

function Shape(props: { kind: "issue" | "pr"; color: keyof Tokens; hollow?: boolean }) {
  return (
    <Canvas
      style={{ width: 14, height: 14 }}
      draw={(cx) => {
        const c = token(props.color) as string;
        cx.beginPath();
        if (props.kind === "issue") cx.arc(7, 7, 5, 0, Math.PI * 2);
        else cx.roundRect(2, 2, 10, 10, 2.5);
        cx.fillStyle = props.hollow ? (token("popover") as string) : c;
        cx.fill();
        cx.strokeStyle = c;
        cx.lineWidth = props.hollow ? 1.75 : 1;
        cx.stroke();
      }}
    />
  );
}

function Edge(props: { color: keyof Tokens; dashed?: boolean }) {
  return (
    <Canvas
      style={{ width: 20, height: 10 }}
      draw={(cx) => {
        const c = token(props.color) as string;
        cx.strokeStyle = c;
        cx.fillStyle = c;
        cx.lineWidth = 1.5;
        cx.setLineDash(props.dashed ? [3, 2.5] : []);
        cx.beginPath();
        cx.moveTo(1, 5);
        cx.lineTo(14, 5);
        cx.stroke();
        cx.setLineDash([]);
        cx.beginPath();
        cx.moveTo(19, 5);
        cx.lineTo(13.5, 2);
        cx.lineTo(13.5, 8);
        cx.closePath();
        cx.fill();
      }}
    />
  );
}
