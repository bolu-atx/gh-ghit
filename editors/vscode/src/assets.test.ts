import * as assert from "assert";
import { assetName } from "./assets";

assert.strictEqual(assetName("darwin", "arm64"), "gh-ghit-darwin-arm64");
assert.strictEqual(assetName("linux", "x64"), "gh-ghit-linux-amd64");
assert.strictEqual(assetName("win32", "x64"), "gh-ghit-windows-amd64.exe");
assert.throws(() => assetName("freebsd", "x64"));
assert.throws(() => assetName("linux", "ia32"));
console.log("ok");
