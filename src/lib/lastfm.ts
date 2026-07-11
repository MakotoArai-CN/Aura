const API_URL = "https://ws.audioscrobbler.com/2.0/";
const API_KEY = "a4e1da9873cb3a0f42c53f3b1d57e9ab"; // public Last.fm API key

function lsGet<T>(key: string): T | null {
  try {
    const v = localStorage.getItem(key);
    return v ? JSON.parse(v) : null;
  } catch {
    return null;
  }
}

export const lastfm = {
  isAuthorized(): boolean {
    return !!lsGet<string>("lastfm_session_key");
  },

  async sendNowPlaying(title: string, artist: string): Promise<void> {
    const sk = lsGet<string>("lastfm_session_key");
    if (!sk) return;
    const params = new URLSearchParams({
      method: "track.updateNowPlaying",
      track: title,
      artist,
      api_key: API_KEY,
      sk,
      format: "json",
    });
    await fetch(API_URL, { method: "POST", body: params });
  },

  async scrobble(playedFrom: number, title: string, artist: string, album: string): Promise<void> {
    const sk = lsGet<string>("lastfm_session_key");
    if (!sk) return;
    const timestamp = Math.floor(playedFrom / 1000);
    const params = new URLSearchParams({
      method: "track.scrobble",
      track: title,
      artist,
      album: album ?? "",
      timestamp: String(timestamp),
      api_key: API_KEY,
      sk,
      format: "json",
    });
    await fetch(API_URL, { method: "POST", body: params });
  },
};
