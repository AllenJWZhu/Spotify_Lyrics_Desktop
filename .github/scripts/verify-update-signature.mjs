// Checks that FILE's updater signature verifies against the public key in
// tauri.conf.json, i.e. that installed copies will accept this update.
// A mismatch between the signing secret and the committed key would
// otherwise only show up when users' updates start failing.
//
//   node verify-update-signature.mjs FILE FILE.sig crates/app/tauri.conf.json
//
// Tauri's .sig and pubkey are base64-wrapped minisign files; minisign signs
// with Ed25519, over the BLAKE2b-512 hash of the file for "ED" signatures.
import { createHash, createPublicKey, verify } from "node:crypto";
import { readFileSync } from "node:fs";

const [file, sigFile, configFile] = process.argv.slice(2);

const unwrap = (b64) => Buffer.from(b64.trim(), "base64").toString("utf8").split("\n");

const pubkey = JSON.parse(readFileSync(configFile, "utf8")).plugins?.updater?.pubkey ?? "";
const keyLines = unwrap(pubkey);
const key = Buffer.from(keyLines[1] ?? "", "base64");
if (key.length !== 42 || key.toString("latin1", 0, 2) !== "Ed") {
  fail(`no valid updater public key in ${configFile}`);
}

const sigLines = unwrap(readFileSync(sigFile, "utf8"));
const sig = Buffer.from(sigLines[1] ?? "", "base64");
const algorithm = sig.toString("latin1", 0, 2);
if (sig.length !== 74 || (algorithm !== "Ed" && algorithm !== "ED")) {
  fail(`${sigFile} isn't a minisign signature`);
}
if (!sig.subarray(2, 10).equals(key.subarray(2, 10))) {
  fail(
    `signed with key ${hex(sig.subarray(2, 10))}, but the app trusts key ${hex(key.subarray(2, 10))}`,
  );
}

const publicKey = createPublicKey({
  key: { kty: "OKP", crv: "Ed25519", x: key.subarray(10).toString("base64url") },
  format: "jwk",
});
const data = readFileSync(file);
const message = algorithm === "ED" ? createHash("blake2b512").update(data).digest() : data;
if (!verify(null, message, publicKey, sig.subarray(10))) {
  fail(`signature doesn't match ${file}`);
}
console.log(`${file}: signature OK (key ${hex(key.subarray(2, 10))})`);

function hex(bytes) {
  return Buffer.from(bytes).reverse().toString("hex").toUpperCase();
}

function fail(message) {
  console.error(`::error::Updater signature check failed: ${message}`);
  process.exit(1);
}
