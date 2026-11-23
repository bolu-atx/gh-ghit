// Small shared pieces: keycaps, label chips, avatars.
import { Image, Text, View } from "nsh";
import type { Label } from "./gh";

export function Kbd(props: { k: string }) {
  return (
    <View class="min-w-[18px] items-center rounded-[4px] border border-border bg-card px-1">
      <Text class="text-[11px] leading-4 text-muted-foreground">{props.k}</Text>
    </View>
  );
}

/** A key hint: keycaps then what they do. */
export function Hint(props: { keys: string[]; label: string }) {
  return (
    <View class="flex-row items-center gap-1">
      {props.keys.map((k) => (
        <Kbd k={k} />
      ))}
      <Text class="text-[11px] text-muted-foreground pl-0.5">{props.label}</Text>
    </View>
  );
}

export function LabelChip(props: { label: Label }) {
  return (
    <View class="flex-row items-center gap-1.5 rounded-full border border-border px-2 py-px">
      <View class="size-2 rounded-full" style={{ backgroundColor: `#${props.label.color}` }} />
      <Text class="text-[11px] text-muted-foreground">{props.label.name}</Text>
    </View>
  );
}

export function Avatar(props: { login: string; size?: number }) {
  const size = props.size ?? 24;
  const login = props.login.replace(/^app\//, "");
  return (
    <Image
      src={`https://github.com/${login}.png?size=${size * 2}`}
      style={{ width: size, height: size, borderRadius: size / 2 }}
      class="bg-muted"
    />
  );
}
