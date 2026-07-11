const OAUTH_URL = "https://github.com/login/oauth";
const API_URL = "https://api.github.com";
const CLIENT_ID = "e099a4803bb1e2e773a3";
const CLIENT_SECRET = "81fbfc45c65af8c0fbf2b4dae6f23f22e656cfb8";

function lsGet<T>(key: string): T | null {
  try {
    const v = localStorage.getItem(key);
    return v ? JSON.parse(v) : null;
  } catch {
    return null;
  }
}

async function githubApi(path: string, options: RequestInit = {}): Promise<Response> {
  const token = lsGet<string>("githubOauthAccessKey");
  return fetch(`${API_URL}${path}`, {
    ...options,
    headers: {
      accept: "application/json",
      ...(token ? { Authorization: `token ${token}` } : {}),
      ...options.headers,
    },
  });
}

export const github = {
  getLoginUrl(): string {
    return `${OAUTH_URL}/authorize?client_id=${CLIENT_ID}&scope=gist`;
  },

  async handleCallback(code: string): Promise<void> {
    const resp = await fetch(`${OAUTH_URL}/access_token`, {
      method: "POST",
      headers: { accept: "application/json", "Content-Type": "application/json" },
      body: JSON.stringify({ client_id: CLIENT_ID, client_secret: CLIENT_SECRET, code }),
    });
    const data = await resp.json();
    if (data.access_token) {
      localStorage.setItem("githubOauthAccessKey", JSON.stringify(data.access_token));
    }
  },

  async getUser(): Promise<{ login: string } | null> {
    const token = lsGet<string>("githubOauthAccessKey");
    if (!token) return null;
    const resp = await githubApi("/user");
    if (!resp.ok) return null;
    return resp.json();
  },

  logout() {
    localStorage.removeItem("githubOauthAccessKey");
  },

  async backupToGist(gistId: string | null, isPublic: boolean): Promise<void> {
    const items: Record<string, unknown> = {};
    for (const key of Object.keys(localStorage)) {
      if (key !== "gistid" && key !== "githubOauthAccessKey") {
        try { items[key] = JSON.parse(localStorage.getItem(key)!); }
        catch { items[key] = localStorage.getItem(key); }
      }
    }

    const files: Record<string, { content: string }> = {
      "listen1_backup.json": { content: JSON.stringify(items) },
    };

    const method = gistId ? "PATCH" : "POST";
    const path = gistId ? `/gists/${gistId}` : "/gists";

    await githubApi(path, {
      method,
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        description: `updated by Listen1 at ${new Date().toLocaleString()}`,
        public: isPublic,
        files,
      }),
    });
  },

  async restoreFromGist(gistId: string): Promise<void> {
    const resp = await githubApi(`/gists/${gistId}`);
    const data = await resp.json();
    const backupFile = data.files?.["listen1_backup.json"];
    if (!backupFile) throw new Error("Not found");

    const content = backupFile.truncated
      ? await (await fetch(backupFile.raw_url)).json()
      : JSON.parse(backupFile.content);

    for (const [key, value] of Object.entries(content)) {
      localStorage.setItem(key, JSON.stringify(value));
    }
  },

  async listBackups(): Promise<Array<{ id: string; description: string }>> {
    const resp = await githubApi("/gists");
    const data = await resp.json();
    return (data as Array<{ id: string; description: string }>).filter(
      (g) => g.description?.startsWith("updated by Listen1")
    );
  },
};
