// First run on the web: the browser cannot ask gh for a token, so the user gives one.
import { signal, Text, View } from "nsh";
import { Input } from "nsh/base";
import { isWeb, saveToken } from "./api";
import { Icon } from "./icons";

const TOKEN_URL = "https://github.com/settings/personal-access-tokens/new";

export function Connect(props: { onConnected: () => void; reason?: string | null }) {
  const value = signal("");
  const error = signal<string | null>(null);
  const busy = signal(false);

  async function submit() {
    if (!value().trim() || busy()) return;
    busy.set(true);
    error.set(null);
    try {
      await saveToken(value());
      props.onConnected();
    } catch (e) {
      error.set(`GitHub did not accept that token: ${String((e as Error).message ?? e)}`);
    } finally {
      busy.set(false);
    }
  }

  return (
    <View class="flex-1 items-center justify-center bg-background px-8">
      <View class="w-full max-w-[440px] gap-5">
        <View class="flex-row items-center gap-3">
          <Icon glyph="map" color="primary" size={22} />
          <Text class="text-[22px] font-semibold text-foreground">Connect to GitHub</Text>
        </View>
        {isWeb ? (
          <>
            <Text class="text-[14px] leading-[22px] text-muted-foreground">
              ghit reads issues and pull requests with your token. In a browser it cannot ask the gh CLI, so paste a
              token here. It stays in this browser's storage and goes only to api.github.com.
            </Text>
            <Text class="text-[13px] leading-5 text-muted-foreground">
              {`Make a fine-grained token at ${TOKEN_URL} with read-only access to Issues, Pull requests and Contents.`}
            </Text>
            <Input
              class="rounded-(--radius) border border-input bg-card px-3 py-2 text-[13px] text-foreground focus:border-ring"
              placeholder="github_pat_…"
              masked
              autoFocus
              autoCorrect={false}
              onChange={(e) => value.set(e.value)}
              onSubmit={() => void submit()}
            />
            <View class="flex-row items-center gap-3">
              <View
                role="button"
                onPress={() => void submit()}
                class={`rounded-(--radius) px-4 py-2 ${value().trim() && !busy() ? "bg-primary" : "bg-muted"}`}
              >
                <Text class={value().trim() && !busy() ? "text-[13px] font-medium text-primary-foreground" : "text-[13px] text-muted-foreground"}>
                  {busy() ? "Checking…" : "Connect"}
                </Text>
              </View>
              {error() ? <Text class="flex-1 text-[12.5px] leading-5 text-shut">{error()}</Text> : null}
            </View>
          </>
        ) : (
          <Text class="text-[14px] leading-[22px] text-muted-foreground">
            {`${props.reason ?? "gh has no token."} Run gh auth login in a terminal, then press r to try again.`}
          </Text>
        )}
      </View>
    </View>
  );
}
