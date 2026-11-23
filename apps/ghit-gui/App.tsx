// ghit-gui: an nsh spike of ghit. Read-only issues, pull requests and their map, over the `gh` CLI.
import { effect, keymap, List, mount, signal, Text, untracked, View } from "nsh";
import type { InputRef, ListRef } from "nsh";
import { Input } from "nsh/base";
import { AuthError, forgetToken, isWeb, loadToken } from "./api";
import { Connect } from "./connect";
import { DetailPane, Message, type Target } from "./detail";
import { type Kind, type Row, listRows, relTime, step } from "./gh";
import { type Glyph, Icon, rowStatus, stateColor, stateGlyph } from "./icons";
import { MapView } from "./map";
import { Hint } from "./ui";

const DEFAULT_REPO = "bolu-atx/gh-ghit";
const PAGE = 10;

type Screen = "issue" | "pr" | "map";

function App() {
  const repo = signal(DEFAULT_REPO);
  const screen = signal<Screen>("issue");
  const rows = signal<Row[]>([]);
  const listError = signal<string | null>(null);
  const loading = signal(false);
  const sel = signal(-1);
  const mapTarget = signal<Target | null>(null);
  const auth = signal<"checking" | "ready" | "needed">("checking");
  const authReason = signal<string | null>(null);
  // Bumped by reload so the map refetches without a repo change.
  const mapEpoch = signal(0);

  let listRef: ListRef | undefined;
  let repoRef: InputRef | undefined;
  let seq = 0;

  const kind = (): Kind => (screen() === "pr" ? "pr" : "issue");

  async function loadList() {
    const my = ++seq;
    loading.set(true);
    listError.set(null);
    try {
      const r = await listRows(repo(), kind());
      if (my !== seq) return;
      rows.set(r);
      sel.set(r.length ? 0 : -1);
    } catch (e) {
      if (my !== seq) return;
      if (e instanceof AuthError) {
        authReason.set(e.message);
        auth.set("needed");
        return;
      }
      rows.set([]);
      sel.set(-1);
      listError.set(String((e as Error).message ?? e));
    } finally {
      if (my === seq) loading.set(false);
    }
  }

  async function checkAuth() {
    auth.set("checking");
    try {
      auth.set((await loadToken()) ? "ready" : "needed");
    } catch (e) {
      authReason.set(String((e as Error).message ?? e));
      auth.set("needed");
    }
  }
  void checkAuth();

  effect(() => {
    repo();
    if (auth() !== "ready" || screen() === "map") return;
    untracked(() => void loadList());
  });

  effect(() => {
    const i = sel();
    if (i >= 0) untracked(() => listRef?.scrollToIndex({ index: i, viewPosition: 0.5, animated: false }));
  });

  const listTarget = (): Target | null => {
    const r = rows()[sel()];
    return r ? { kind: kind(), number: r.number } : null;
  };

  const reload = () => {
    if (auth() !== "ready") return void checkAuth();
    if (screen() === "map") mapEpoch.update((n) => n + 1);
    else void loadList();
  };
  const mapRepo = () => {
    mapEpoch();
    return repo();
  };

  keymap.bind(
    {
      "1": () => screen.set("issue"),
      "2": () => screen.set("pr"),
      "3": () => screen.set("map"),
      "/": () => repoRef?.focus(),
      r: reload,
    },
    { when: "notEditing" },
  );

  return (
    <View class="flex-1 flex-row bg-background">
      <Rail screen={screen()} onScreen={(v) => screen.set(v)} onReload={reload} />

      <View class="flex-1">
        <View class="h-12 flex-row items-center gap-2.5 px-4 border-b border-border bg-sidebar">
          <Icon glyph="repo" color="muted-foreground" size={15} />
          <Input
            ref={(r) => (repoRef = r)}
            class="w-72 text-[13px] font-medium text-foreground rounded-[4px] px-1.5 py-1 focus:bg-background"
            defaultValue={DEFAULT_REPO}
            placeholder="owner/repo"
            autoCorrect={false}
            onSubmit={(e) => {
              repo.set(e.text.trim() || DEFAULT_REPO);
              repoRef?.blur();
            }}
          />
          <View class="flex-1" />
          <Text class="text-[12px] text-muted-foreground">{loading() ? "Loading…" : ""}</Text>
          {isWeb && auth() === "ready" ? (
            <View
              role="button"
              onPress={() => void forgetToken().then(() => auth.set("needed"))}
              class="rounded-(--radius) px-2 py-1 hover:bg-sidebar-accent"
            >
              <Text class="text-[12px] text-muted-foreground">Disconnect</Text>
            </View>
          ) : null}
        </View>

        {auth() === "checking" ? (
          <View class="flex-1" />
        ) : auth() === "needed" ? (
          <Connect reason={authReason()} onConnected={() => auth.set("ready")} />
        ) : screen() === "map" ? (
          <View class="flex-1 flex-row">
            <MapView repo={mapRepo} selected={mapTarget} onSelect={(t) => mapTarget.set(t)} />
            {mapTarget() ? (
              <View class="w-[520px] border-l border-border">
                <DetailPane repo={repo} target={mapTarget} />
              </View>
            ) : null}
          </View>
        ) : (
          <View class="flex-1 flex-row">
            <ListKeys move={(d) => sel.set(step(sel(), d, rows().length))} />
            <View class="w-[380px] border-r border-border bg-sidebar">
              <View class="flex-row items-baseline gap-2 px-5 pt-4 pb-2.5 border-b border-border">
                <Text class="text-[15px] font-semibold text-foreground">
                  {kind() === "pr" ? "Open pull requests" : "Open issues"}
                </Text>
                <Text class="text-[13px] text-muted-foreground">{loading() ? "" : String(rows().length)}</Text>
              </View>
              {listError() ? (
                <Message
                  title={`${kind() === "pr" ? "Pull requests" : "Issues"} did not load`}
                  body={`${listError()}\nCheck the repository name, or run gh auth login.`}
                />
              ) : (
                <List
                  class="flex-1"
                  each={rows()}
                  keyExtractor={(r) => String(r.number)}
                  ref={(r) => (listRef = r)}
                  empty={
                    <View class="px-5 py-6">
                      <Text class="text-[13px] text-muted-foreground">
                        {loading() ? "" : `No open ${kind() === "pr" ? "pull requests" : "issues"} in ${repo()}.`}
                      </Text>
                    </View>
                  }
                >
                  {(item, index) => (
                    <ItemRow row={item} kind={kind()} selected={() => index() === sel()} onPress={() => sel.set(index())} />
                  )}
                </List>
              )}
            </View>
            <DetailPane repo={repo} target={listTarget} />
          </View>
        )}

        <StatusBar screen={screen()} hasTarget={!!mapTarget()} />
      </View>
    </View>
  );
}

/** List movement keys, bound only while the list shows (the map binds j and k itself). */
function ListKeys(props: { move: (d: number) => void }) {
  keymap.bind(
    {
      j: () => props.move(1),
      down: () => props.move(1),
      k: () => props.move(-1),
      up: () => props.move(-1),
      "ctrl+d": () => props.move(PAGE),
      "ctrl+u": () => props.move(-PAGE),
      g: () => props.move(-1e9),
      "shift+g": () => props.move(1e9),
    },
    { when: "notEditing", repeat: true },
  );
  return null;
}

function Rail(props: { screen: Screen; onScreen: (v: Screen) => void; onReload: () => void }) {
  const items: { v: Screen; glyph: Glyph; label: string; key: string }[] = [
    { v: "issue", glyph: "list-issues", label: "Issues", key: "1" },
    { v: "pr", glyph: "list-prs", label: "Pulls", key: "2" },
    { v: "map", glyph: "map", label: "Map", key: "3" },
  ];
  return (
    <View class="w-[68px] items-center gap-1 pt-3 pb-3 bg-sidebar border-r border-border">
      {items.map((it) => (
        <View
          role="button"
          accessibilityLabel={`${it.label} (${it.key})`}
          onPress={() => props.onScreen(it.v)}
          class={`w-14 items-center gap-1 rounded-(--radius) py-2 ${props.screen === it.v ? "bg-sidebar-accent" : "hover:bg-sidebar-accent/60"}`}
        >
          <Icon glyph={it.glyph} color={props.screen === it.v ? "primary" : "muted-foreground"} size={18} />
          <Text class={`text-[10.5px] ${props.screen === it.v ? "text-foreground font-medium" : "text-muted-foreground"}`}>
            {it.label}
          </Text>
        </View>
      ))}
      <View class="flex-1" />
      <View
        role="button"
        accessibilityLabel="Reload (r)"
        onPress={props.onReload}
        class="w-14 items-center gap-1 rounded-(--radius) py-2 hover:bg-sidebar-accent/60"
      >
        <Icon glyph="reload" color="muted-foreground" size={16} />
        <Text class="text-[10.5px] text-muted-foreground">Reload</Text>
      </View>
    </View>
  );
}

function ItemRow(props: { row: () => Row; kind: Kind; selected: () => boolean; onPress: () => void }) {
  const r = props.row;
  const status = () => rowStatus(props.kind, r().state, r().isDraft, r().stateReason);
  return (
    <View
      onPress={props.onPress}
      class={`flex-row gap-3 pl-5 pr-4 py-3 border-b border-sidebar-border ${props.selected() ? "bg-sidebar-accent" : "hover:bg-sidebar-accent/40"}`}
    >
      {props.selected() ? <View class="absolute left-0 top-0 bottom-0 w-[3px] bg-primary" /> : null}
      <View class="pt-px">
        <Icon glyph={() => stateGlyph(props.kind, status())} color={() => stateColor(status())} size={16} />
      </View>
      <View class="flex-1 gap-1">
        <View class="flex-row gap-3">
          <Text class="flex-1 text-[13px] leading-[18px] font-medium text-foreground truncate">{r().title}</Text>
          <Text class="text-[11.5px] leading-[18px] text-muted-foreground">{relTime(r().updatedAt)}</Text>
        </View>
        <View class="flex-row items-center gap-2">
          <Text class="text-[12px] text-muted-foreground">{`#${r().number}`}</Text>
          <Text class="text-[12px] text-muted-foreground truncate">
            {`${r().isDraft ? "Draft by " : ""}${(r().author?.login ?? "ghost").replace(/^app\//, "")}`}
          </Text>
          <View class="flex-1" />
          <View class="flex-row items-center gap-1">
            {r()
              .labels.slice(0, 4)
              .map((l) => (
                <View class="size-2 rounded-full" style={{ backgroundColor: `#${l.color}` }} />
              ))}
          </View>
        </View>
      </View>
    </View>
  );
}

function StatusBar(props: { screen: Screen; hasTarget: boolean }) {
  return (
    <View class="h-8 flex-row items-center gap-5 px-4 border-t border-border bg-sidebar">
      {props.screen === "map" ? (
        <>
          <Hint keys={["j", "k"]} label="step through items" />
          <Hint keys={["f"]} label="fit" />
          <Hint keys={["u"]} label="unlinked" />
          <Hint keys={["−", "="]} label="zoom" />
          {props.hasTarget ? <Hint keys={["esc"]} label="close detail" /> : null}
        </>
      ) : (
        <>
          <Hint keys={["j", "k"]} label="move" />
          <Hint keys={["g", "G"]} label="first, last" />
          <Hint keys={["space"]} label="page down" />
          {props.screen === "pr" ? <Hint keys={["tab"]} label="conversation or files" /> : null}
        </>
      )}
      <View class="flex-1" />
      <Hint keys={["1", "2", "3"]} label="views" />
      <Hint keys={["/"]} label="repository" />
      <Hint keys={["r"]} label="reload" />
    </View>
  );
}

mount(App);
