// The reading pane: one issue or PR, its conversation, and for PRs the diff.
import { effect, keymap, List, Scroll, signal, Text, untracked, View } from "nsh";
import type { ListRef, ScrollRef } from "nsh";
import type { JSX } from "nsh/jsx-runtime";
import { TextView } from "nsh/base";
import { type Detail, type DiffLine, type Kind, parseDiff, prDiff, relTime, viewDetail } from "./gh";
import { Icon, rowStatus, stateColor, stateGlyph } from "./icons";
import { Avatar, LabelChip } from "./ui";

export interface Target {
  kind: Kind;
  number: number;
}

type Pane = "conversation" | "files";

const SCROLL_STEP = 160;

const STATUS_WORD = { open: "Open", done: "Closed", shut: "Closed", draft: "Draft" } as const;

const ROW_BG: Record<DiffLine["kind"], string> = {
  file: "bg-muted border-y border-border",
  hunk: "bg-accent/50",
  add: "bg-add-bg",
  del: "bg-del-bg",
  ctx: "",
};

const MARKDOWN_CLASSES = {
  heading1: "text-[19px] font-semibold",
  heading2: "text-[16px] font-semibold",
  heading3: "text-[15px] font-semibold",
  paragraph: "text-[14px] leading-[22px]",
  code: "font-mono text-[12.5px] bg-muted rounded-[4px] px-1",
  link: "text-primary",
};

export function DetailPane(props: { repo: () => string; target: () => Target | null }) {
  const detail = signal<Detail | null>(null);
  const error = signal<string | null>(null);
  const pane = signal<Pane>("conversation");
  const diff = signal<DiffLine[]>([]);
  const diffState = signal<"idle" | "loading" | "ready" | "error">("idle");
  let seq = 0;
  let y = 0;
  let bodyRef: ScrollRef | undefined;
  let diffRef: ListRef | undefined;

  async function load(t: Target) {
    const my = ++seq;
    detail.set(null);
    error.set(null);
    diff.set([]);
    diffState.set("idle");
    y = 0;
    if (t.kind !== "pr") pane.set("conversation");
    try {
      const d = await viewDetail(props.repo(), t.kind, t.number);
      if (my === seq) detail.set(d);
    } catch (e) {
      if (my === seq) error.set(String((e as Error).message ?? e));
    }
    if (my === seq && t.kind === "pr" && pane() === "files") void loadDiff(t.number);
  }

  async function loadDiff(n: number) {
    if (diffState() !== "idle") return;
    const my = seq;
    diffState.set("loading");
    try {
      const raw = await prDiff(props.repo(), n);
      if (my !== seq) return;
      diff.set(parseDiff(raw));
      diffState.set("ready");
    } catch {
      if (my === seq) diffState.set("error");
    }
  }

  effect(() => {
    const t = props.target();
    if (t) untracked(() => void load(t));
    else detail.set(null);
  });

  function show(p: Pane) {
    const t = props.target();
    if (!t || (p === "files" && t.kind !== "pr")) return;
    pane.set(p);
    y = 0;
    if (p === "files") void loadDiff(t.number);
  }

  function scroll(delta: number) {
    y = Math.max(0, y + delta);
    (pane() === "files" ? diffRef : bodyRef)?.scrollTo({ y, animated: true });
  }

  keymap.bind(
    {
      tab: () => show(pane() === "files" ? "conversation" : "files"),
      "shift+j": () => scroll(SCROLL_STEP),
      "shift+k": () => scroll(-SCROLL_STEP),
      space: () => scroll(SCROLL_STEP * 3),
      "shift+space": () => scroll(-SCROLL_STEP * 3),
    },
    { when: "notEditing" },
  );

  return (
    <View class="flex-1 bg-background">
      {error() ? (
        <Message title="This item did not load" body={error()!} />
      ) : !props.target() ? (
        <Message title="Nothing selected" body="Pick an item from the list, or press j and k to move through it." />
      ) : !detail() ? (
        <Skeleton />
      ) : (
        <View class="flex-1">
          <Header d={detail()!} kind={props.target()!.kind} pane={pane()} onPane={show} />
          {pane() === "files" ? (
            <DiffView state={diffState()} lines={diff()} refFn={(r) => (diffRef = r)} />
          ) : (
            <Scroll class="flex-1" ref={(r: ScrollRef) => (bodyRef = r)}>
              <View class="px-8 pt-5 pb-16 gap-6 max-w-[780px]">
                <Entry login={detail()!.author?.login ?? "ghost"} verb="opened this" at={detail()!.createdAt}>
                  <TextView
                    markdown={detail()!.body.trim() || "*No description.*"}
                    classes={MARKDOWN_CLASSES}
                    selectable
                    links="open"
                  />
                </Entry>
                {detail()!.comments.map((c) => (
                  <Entry login={c.author?.login ?? "ghost"} verb="commented" at={c.createdAt}>
                    <TextView markdown={c.body} classes={MARKDOWN_CLASSES} selectable links="open" />
                  </Entry>
                ))}
              </View>
            </Scroll>
          )}
        </View>
      )}
    </View>
  );
}

function Header(props: { d: Detail; kind: Kind; pane: Pane; onPane: (p: Pane) => void }) {
  const d = props.d;
  const status = rowStatus(props.kind, d.state, d.isDraft, d.stateReason);
  const word = props.kind === "pr" && status === "done" ? "Merged" : STATUS_WORD[status];
  const who = d.author?.login.replace(/^app\//, "") ?? "ghost";
  return (
    <View class="px-8 pt-5 gap-3 border-b border-border">
      <View class="flex-row items-center gap-2.5">
        <View class="flex-row items-center gap-1.5 rounded-full px-2.5 py-1 bg-muted">
          <Icon glyph={stateGlyph(props.kind, status)} color={stateColor(status)} size={14} />
          <Text class="text-[12px] font-medium text-foreground">{word}</Text>
        </View>
        <Text class="text-[13px] text-muted-foreground">
          {props.kind === "pr"
            ? `${who} wants to merge ${d.headRefName} into ${d.baseRefName}`
            : `${who} opened this ${relTime(d.createdAt)} ago`}
        </Text>
      </View>
      <Text class="text-[22px] leading-[28px] font-semibold text-foreground max-w-[780px]">
        {d.title}
        <Text class="text-muted-foreground font-normal">{`  #${d.number}`}</Text>
      </Text>
      {d.labels.length ? (
        <View class="flex-row flex-wrap gap-1.5">
          {d.labels.map((l) => (
            <LabelChip label={l} />
          ))}
        </View>
      ) : null}
      <View class="flex-row gap-5 pt-1">
        <TabButton active={props.pane === "conversation"} onPress={() => props.onPane("conversation")}>
          <Icon glyph="comment" color={props.pane === "conversation" ? "foreground" : "muted-foreground"} size={14} />
          <Text class={props.pane === "conversation" ? "text-[13px] text-foreground font-medium" : "text-[13px] text-muted-foreground"}>
            Conversation
          </Text>
          <Text class="text-[12px] text-muted-foreground">{String(d.comments.length)}</Text>
        </TabButton>
        {props.kind === "pr" ? (
          <TabButton active={props.pane === "files"} onPress={() => props.onPane("files")}>
            <Text class={props.pane === "files" ? "text-[13px] text-foreground font-medium" : "text-[13px] text-muted-foreground"}>
              Files changed
            </Text>
            <Text class="text-[12px] text-open">{`+${d.additions ?? 0}`}</Text>
            <Text class="text-[12px] text-shut">{`−${d.deletions ?? 0}`}</Text>
          </TabButton>
        ) : null}
      </View>
    </View>
  );
}

function TabButton(props: { active: boolean; onPress: () => void; children?: JSX.Element }) {
  return (
    <View
      role="button"
      onPress={props.onPress}
      class={`flex-row items-center gap-1.5 pb-2.5 border-b-2 ${props.active ? "border-primary" : "border-transparent"}`}
    >
      {props.children}
    </View>
  );
}

function Entry(props: { login: string; verb: string; at: string; children?: JSX.Element }) {
  const who = props.login.replace(/^app\//, "");
  return (
    <View class="flex-row gap-3">
      <Avatar login={props.login} size={28} />
      <View class="flex-1 gap-1.5">
        <Text class="text-[13px] text-muted-foreground leading-[28px]">
          <Text class="font-semibold text-foreground">{who}</Text>
          {` ${props.verb} ${relTime(props.at)} ago`}
        </Text>
        {props.children}
      </View>
    </View>
  );
}

function DiffView(props: { state: string; lines: DiffLine[]; refFn: (r: ListRef) => void }) {
  if (props.state === "loading" || props.state === "idle") return <Skeleton rows={14} mono />;
  if (props.state === "error") return <Message title="The diff did not load" body="gh pr diff failed. Press Tab twice to try again." />;
  if (!props.lines.length) return <Message title="No changes" body="This pull request has an empty diff." />;
  const sticky = props.lines.filter((l) => l.kind === "file").map((l) => l.id);
  return (
    <List
      class="flex-1"
      each={props.lines}
      keyExtractor={(l) => String(l.id)}
      stickyHeaderIndices={sticky}
      ref={props.refFn}
      contentClass="pb-16"
    >
      {(line) => <DiffRow line={line} />}
    </List>
  );
}

function DiffRow(props: { line: () => DiffLine }) {
  const l = props.line;
  return (
    <View class={`flex-row ${ROW_BG[l().kind]}`}>
      {l().kind === "file" ? (
        <Text class="px-4 py-2 text-[12.5px] font-semibold text-foreground">{l().text}</Text>
      ) : (
        <>
          <Text class="w-12 pr-2 text-right font-mono text-[11.5px] leading-5 text-muted-foreground">
            {l().old !== undefined ? String(l().old) : ""}
          </Text>
          <Text class="w-12 pr-2 text-right font-mono text-[11.5px] leading-5 text-muted-foreground">
            {l().new !== undefined ? String(l().new) : ""}
          </Text>
          <Text
            class={`w-5 text-center font-mono text-[12px] leading-5 ${l().kind === "add" ? "text-open" : l().kind === "del" ? "text-shut" : "text-muted-foreground"}`}
          >
            {l().kind === "add" ? "+" : l().kind === "del" ? "−" : ""}
          </Text>
          <Text
            class={`flex-1 pr-4 font-mono text-[12px] leading-5 truncate ${l().kind === "hunk" ? "text-muted-foreground" : "text-foreground"}`}
          >
            {l().kind === "hunk" ? `@@ ${l().text}` : l().text || " "}
          </Text>
        </>
      )}
    </View>
  );
}

export function Message(props: { title: string; body: string }) {
  return (
    <View class="flex-1 items-center justify-center px-10">
      <View class="gap-1.5 max-w-[420px] items-center">
        <Text class="text-[15px] font-semibold text-foreground">{props.title}</Text>
        <Text class="text-[13px] leading-5 text-muted-foreground text-center">{props.body}</Text>
      </View>
    </View>
  );
}

function Skeleton(props: { rows?: number; mono?: boolean }) {
  const widths = [72, 94, 60, 88, 40, 96, 78, 52, 90, 66, 84, 30, 70, 92];
  return (
    <View class="px-8 pt-6 gap-3">
      {widths.slice(0, props.rows ?? 8).map((w, i) => (
        <View
          class={`${props.mono ? "h-3" : i === 0 ? "h-6" : "h-3.5"} rounded-[3px] bg-muted`}
          style={{ width: `${props.mono ? w * 0.8 : i === 0 ? 55 : w}%` }}
        />
      ))}
    </View>
  );
}
