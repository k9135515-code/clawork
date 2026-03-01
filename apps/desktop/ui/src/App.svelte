<script lang="ts">
  import { onMount } from 'svelte';

  type Status = {
    app_mode: Record<string, unknown>;
    daemon_running: boolean;
    last_heartbeat: string | null;
    loaded_skills: number;
    adapters: Record<string, boolean>;
  };

  type Briefing = {
    overview: string;
    suggestions: Array<{ at: string; text: string }>;
  };

  let status: Status | null = null;
  let briefing: Briefing | null = null;
  let logs: Array<{ timestamp: string; action: string; decision: string }> = [];
  let prompt = '';
  let running = false;
  let output = '';
  let info = '起動中...';

  const quickExamples = [
    'statusを表示して',
    'briefingを見せて',
    'search rust tauri mcp',
    'browse github.com',
    'artifact 週次メモ :: 今週の進捗を整理して',
    'workspaceの接続状況を確認して'
  ];

  async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(cmd, args);
  }

  async function refresh() {
    try {
      status = await invoke<Status>('get_status');
      logs = await invoke('get_audit_events', { limit: 10 });
      briefing = await invoke('get_daily_briefing');
      info = status.daemon_running ? 'Daemon is running.' : 'Daemon is stopped.';
    } catch (e) {
      info = `Desktop API unavailable in browser preview: ${String(e)}`;
    }
  }

  async function runPrompt(dryRun = false) {
    const text = prompt.trim();
    if (!text || running) return;
    running = true;
    output = '';
    try {
      const res = await invoke<Record<string, unknown>>('nl_execute', {
        instruction: text,
        dry_run: dryRun,
        continue_on_error: true,
        approval_token: null
      });
      output = JSON.stringify(res, null, 2);
      await refresh();
    } catch (e) {
      output = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    } finally {
      running = false;
    }
  }

  function pickExample(example: string) {
    prompt = example;
  }

  onMount(() => {
    refresh();
    const id = setInterval(refresh, 15000);
    return () => clearInterval(id);
  });
</script>

<div class="layout">
  <aside class="sidebar">
    <div class="brand">Clawork</div>

    <nav class="menu">
      <button class="menu-item active" type="button">Computer</button>
      <button class="menu-item" type="button">新しいタスク</button>
      <button class="menu-item" type="button">タスク</button>
      <button class="menu-item" type="button">ファイル</button>
      <button class="menu-item" type="button">コネクタ</button>
      <button class="menu-item" type="button">ライブ例</button>
    </nav>

    <div class="status-block">
      <div class="status-line">{info}</div>
      {#if status}
        <div class="meta">skills: {status.loaded_skills}</div>
        <div class="meta">last heartbeat: {status.last_heartbeat ?? 'n/a'}</div>
      {/if}
    </div>
  </aside>

  <main class="main">
    <header class="hero">
      <p class="hero-kicker">Clawork Computer</p>
      <h1>自然言語で操作します。</h1>
      <p class="hero-sub">AIが計画を作り、安全確認を通して、必要な処理を順に実行します。</p>
    </header>

    <section class="composer">
      <textarea
        bind:value={prompt}
        placeholder="次は何をしますか？ 例: browse github.com"
      ></textarea>
      <div class="composer-actions">
        <button type="button" class="ghost" on:click={() => runPrompt(true)} disabled={running}>Dry Run</button>
        <button type="button" class="primary" on:click={() => runPrompt(false)} disabled={running}>
          {running ? '実行中...' : '実行'}
        </button>
      </div>
    </section>

    <section class="examples">
      <div class="section-head">
        <h2>タスクの例</h2>
      </div>
      <div class="example-list">
        {#each quickExamples as ex}
          <button type="button" class="example-card" on:click={() => pickExample(ex)}>
            {ex}
          </button>
        {/each}
      </div>
    </section>

    <section class="grid">
      <article class="panel">
        <h3>実行結果</h3>
        <pre>{output || 'ここに実行結果が表示されます。'}</pre>
      </article>
      <article class="panel">
        <h3>Briefing</h3>
        {#if briefing}
          <p>{briefing.overview}</p>
          <ul>
            {#each briefing.suggestions.slice(0, 3) as s}
              <li>{s.text}</li>
            {/each}
          </ul>
        {:else}
          <p>未取得</p>
        {/if}
      </article>
      <article class="panel">
        <h3>最近の監査ログ</h3>
        <ul>
          {#each logs as log}
            <li>{log.action} / {log.decision}</li>
          {/each}
        </ul>
      </article>
    </section>
  </main>
</div>
