import { Icon } from "./ui/Icons";
import "./SplashScreen.css";
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useConnectionStore } from "../stores/connectionStore";
import { LOCAL_BASE } from "../lib/runtimeTransport";
import { useStartupStore } from "../stores/startupStore";

type Server = { id: string; name: string; publicUrl: string };

export default function SplashScreen({ onConnect }: { onConnect: () => void }) {
  const [profiles, setProfiles] = useState<{ id: string; label: string }[]>([]);
  const [profile, setProfile] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [loadingServers, setLoadingServers] = useState(false);
  const [signedIn, setSignedIn] = useState(false);
  const [checkingAccount, setCheckingAccount] = useState(true);
  const [url, setUrl] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [servers, setServers] = useState<Server[]>([]);

  function accountChanged(value: boolean) {
    setSignedIn(value);
    useConnectionStore.getState().setFlamelSignedIn(value);
  }
  async function loadServers() {
    setLoadingServers(true);
    setError("");
    try {
      setServers(await invoke<Server[]>("cloud_servers"));
    } catch {
      setError("Could not load your servers. Try again or work locally.");
    } finally {
      setLoadingServers(false);
    }
  }
  useEffect(() => {
    useStartupStore.getState().hide();
    let active = true;
    invoke<{ id: string; label: string }[]>("runtime_profiles")
      .then(value => { if (active) setProfiles(value); })
      .catch(() => {});
    // Read the active native account, including after switching workspaces.
    // Credentials never enter renderer storage.
    invoke<boolean>("cloud_signed_in")
      .then(value => {
        if (!active) return;
        accountChanged(value);
        if (value) void loadServers();
      })
      .catch(() => {})
      .finally(() => { if (active) setCheckingAccount(false); });
    return () => { active = false; };
  }, []);

  async function perform(action: () => Promise<void>) {
    setBusy(true);
    setError("");
    try { await action(); }
    catch (error) { setError(String(error)); }
    finally { setBusy(false); }
  }
  function connected(base: string, mode: "local" | "remote") {
    const connection = useConnectionStore.getState();
    if (mode === "local") connection.setLocalApiBase(base);
    else connection.setRemoteUrl(base);
    connection.setMode(mode);
    connection.setConnected(true);
    onConnect();
  }
  async function openLocal() {
    await perform(async () => {
      if (profiles.length > 1) {
        if (!profile) throw new Error("Choose a local profile.");
        await invoke("runtime_select_profile", { profile });
      }
      await invoke("cmd_start_local_server");
      connected(LOCAL_BASE, "local");
    });
  }
  const disabled = busy || checkingAccount;
  const localOption = <>
    {profiles.length > 1 && <label>Local profile
      <select value={profile} onChange={event => setProfile(event.target.value)}>
        <option value="">Choose a profile</option>
        {profiles.map(value => <option key={value.id} value={value.id}>{value.label}</option>)}
      </select>
    </label>}
    <button disabled={disabled} onClick={() => void openLocal()}>
      {signedIn ? "Work locally" : "Open local workspace"}
    </button>
  </>;

  return <main className="splash-page">
    <header className="splash-titlebar" data-tauri-drag-region>
      <span data-tauri-drag-region>Flamel Desktop</span>
      <div className="splash-window-controls">
        <button type="button" aria-label="Minimize" onClick={() => void invoke("cmd_minimize")}>−</button>
        <button type="button" aria-label="Maximize" onClick={() => void invoke("cmd_maximize")}>□</button>
        <button type="button" aria-label="Close" onClick={() => void invoke("cmd_close_window")}>×</button>
      </div>
    </header>
    <section className="splash-card" aria-labelledby="splash-heading" aria-busy={disabled}>
      <div className="splash-brand"><Icon.Logo size={36} /><span>flamel<span className="splash-product">desktop</span></span></div>
      {signedIn ? <>
        <h1 id="splash-heading">Your workspaces</h1>
        <p className="splash-intro">Signed in with Flamel</p>
        <div className="splash-local-label">YOUR SERVERS</div>
        <div className="splash-servers" aria-busy={loadingServers}>
          {loadingServers && <p className="splash-hint" role="status">Loading servers…</p>}
          {!loadingServers && servers.length === 0 && !error && <p className="splash-empty">No servers linked to your account.</p>}
          {servers.map(server => <button className="splash-server" key={server.id} disabled={disabled || loadingServers}
            onClick={() => void perform(async () => {
              const base = await invoke<string>("cloud_connect", { server: server.id });
              connected(base, "remote");
            })}>
            <span>{server.name}</span><small>{server.publicUrl}</small>
          </button>)}
        </div>
        {error && <button disabled={disabled || loadingServers} onClick={() => void loadServers()}>Refresh servers</button>}
        <div className="splash-divider"><span>ON THIS DEVICE</span></div>
        {localOption}
        <button className="splash-text-button" disabled={disabled || loadingServers} onClick={() => void perform(async () => {
          await invoke("remote_logout");
          accountChanged(false);
          setServers([]);
        })}>Sign out</button>
      </> : <>
        <h1 id="splash-heading">Open a workspace</h1>
        <p className="splash-intro">Work locally or connect to a server.</p>
        <button className="splash-primary" disabled={disabled} onClick={() => void perform(async () => {
          await invoke("cloud_login", { remember: false });
          accountChanged(true);
          await loadServers();
        })}>{busy ? "Signing in…" : "Sign in with Flamel"}</button>
        {localOption}
        <details>
          <summary>Connect to a server</summary>
          <form onSubmit={event => {
            event.preventDefault();
            void perform(async () => {
              const base = await invoke<string>("remote_login", { url, email, password, remember: false });
              setPassword("");
              connected(base, "remote");
            });
          }} className="splash-server-form">
            <label>Server URL<input type="url" required placeholder="https://your-server.example" value={url} onChange={event => setUrl(event.target.value)} /></label>
            <label>Email<input type="email" required autoComplete="username" value={email} onChange={event => setEmail(event.target.value)} /></label>
            <label>Password<input type="password" required autoComplete="current-password" value={password} onChange={event => setPassword(event.target.value)} /></label>
            <button disabled={disabled}>Connect</button>
          </form>
        </details>
      </>}
      {error && <p className="splash-error" role="alert">{error}</p>}
    </section>
  </main>;
}
