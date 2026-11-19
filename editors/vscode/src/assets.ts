export function assetName(platform: string, arch: string): string {
  const os = ({ darwin: "darwin", linux: "linux", win32: "windows" } as Record<string, string>)[platform];
  const cpu = ({ x64: "amd64", arm64: "arm64" } as Record<string, string>)[arch];
  if (!os || !cpu) {
    throw new Error(`ghit has no release binary for ${platform}/${arch}.`);
  }
  return `gh-ghit-${os}-${cpu}${os === "windows" ? ".exe" : ""}`;
}
