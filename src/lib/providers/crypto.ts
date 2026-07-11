// NetEase crypto — uses node-forge (browser build)
import forge from "node-forge";

function createSecretKey(size: number): string {
  const choice = "012345679abcdef".split("");
  const result: string[] = [];
  for (let i = 0; i < size; i++) {
    result.push(choice[Math.floor(Math.random() * choice.length)]);
  }
  return result.join("");
}

function aesEncrypt(text: string, secKey: string, algo: "AES-CBC" | "AES-ECB"): forge.util.ByteStringBuffer {
  const cipher = forge.cipher.createCipher(algo, secKey);
  cipher.start({ iv: "0102030405060708" });
  cipher.update(forge.util.createBuffer(text));
  cipher.finish();
  return cipher.output;
}

function rsaEncrypt(text: string, pubKey: string, modulus: string): string {
  const reversed = text.split("").reverse().join("");
  const n = new (forge as unknown as { jsbn: { BigInteger: new (s: string, base: number) => bigint } }).jsbn.BigInteger(modulus, 16);
  const e = new (forge as unknown as { jsbn: { BigInteger: new (s: string, base: number) => bigint } }).jsbn.BigInteger(pubKey, 16);
  const b = new (forge as unknown as { jsbn: { BigInteger: new (s: string, base: number) => bigint } }).jsbn.BigInteger(
    forge.util.bytesToHex(reversed),
    16
  );
  return (b as unknown as { modPow: (e: bigint, n: bigint) => { toString: (base: number) => string } })
    .modPow(e, n)
    .toString(16)
    .padStart(256, "0");
}

export function weapi(obj: object): Record<string, string> {
  const modulus =
    "00e0b509f6259df8642dbc35662901477df22677ec152b5ff68ace615bb7b72" +
    "5152b3ab17a876aea8a5aa76d2e417629ec4ee341f56135fccf695280104e0312ecbd" +
    "a92557c93870114af6c9d05c4f7f0c3685b7a46bee255932575cce10b424d813cfe48" +
    "75d3e82047b97ddef52741d546b8e289dc6935b3ece0462db0a22b8e7";
  const nonce = "0CoJUm6Qyw8W8jud";
  const pubKey = "010001";
  const text = JSON.stringify(obj);
  const secKey = createSecretKey(16);
  const encText = btoa(
    aesEncrypt(btoa(aesEncrypt(text, nonce, "AES-CBC").data), secKey, "AES-CBC").data
  );
  const encSecKey = rsaEncrypt(secKey, pubKey, modulus);
  return { params: encText, encSecKey };
}

export function eapi(url: string, obj: object | string): Record<string, string> {
  const eapiKey = "e82ckenh8dichen8";
  const text = typeof obj === "object" ? JSON.stringify(obj) : obj;
  const message = `nobody${url}use${text}md5forencrypt`;
  const digest = forge.md5
    .create()
    .update(forge.util.encodeUtf8(message))
    .digest()
    .toHex();
  const data = `${url}-36cd479b6b5-${text}-36cd479b6b5-${digest}`;
  return {
    params: aesEncrypt(data, eapiKey, "AES-ECB").toHex().toUpperCase(),
  };
}
