declare module "howler" {
  export class Howl {
    constructor(options: Record<string, unknown>);
    play(): void;
    pause(): void;
    stop(): void;
    unload(): void;
    playing(): boolean;
    volume(): number;
    volume(value: number): void;
    fade(from: number, to: number, duration: number): void;
    seek(): number;
    seek(position: number): void;
    duration(): number;
  }

  export const Howler: {
    volume(): number;
    volume(value: number): void;
    mute(value: boolean): void;
  };
}

declare module "node-forge" {
  namespace forge {
    namespace util {
      interface ByteStringBuffer {
        data: string;
        toHex(): string;
      }
      function createBuffer(input: string): ByteStringBuffer;
      function encodeUtf8(input: string): string;
      function bytesToHex(input: string): string;
    }

    namespace cipher {
      interface Cipher {
        start(options?: { iv?: string }): void;
        update(buffer: util.ByteStringBuffer): void;
        finish(): boolean;
        output: util.ByteStringBuffer;
      }
      function createCipher(algo: string, key: string): Cipher;
    }

    const md5: {
      create(): {
        update(input: string): {
          digest(): {
            toHex(): string;
          };
        };
      };
    };
  }

  const forge: typeof forge;
  export default forge;
}

interface Window {
  l1Player?: unknown;
  __LISTEN1_STREAM_BASE_URL__?: string;
}
