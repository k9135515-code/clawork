<script lang="ts">
  import { onMount } from 'svelte';

  type Status = {
    app_mode: Record<string, unknown>;
    daemon_running: boolean;
    last_heartbeat: string | null;
    loaded_skills: number;
    adapters: Record<string, boolean>;
    operator?: {
      active_sessions: number;
      pending_approvals: number;
      scheduled_tasks: number;
      connectors_health: Record<string, boolean>;
    } | null;
  };

  let status: Status | null = null;
  let tasks: Array<{ id: string; label: string; next_run: string | null }> = [];
  let skills: Array<{ id: string; name: string; abi_version: string }> = [];
  let logs: Array<{ timestamp: string; action: string; decision: string; reason?: string }> = [];
  let ttl = 300;
  let token = '';
  let skillId = 'echo';
  let payload = '{"message":"hello from dashboard"}';
  let newSkillId = 'generated_echo';
  let newSkillName = 'Generated Echo';
  let memoryText = 'Remember this note from dashboard.';
  let memoryQuery = 'dashboard note';
  let memoryHits: Array<{ record: { id: string; text: string }; score: number }> = [];
  let briefing: null | {
    generated_at: string;
    overview: string;
    tasks_due: number;
    recent_audit_events: number;
    memory_entries: number;
    suggestions: Array<{ at: string; text: string }>;
  } = null;
  let operatorSessions: Array<{ id: string; title: string; goal: string; state: string }> = [];
  let selectedSessionId = '';
  let operatorTimeline: Array<{ at: string; kind: string; summary: string }> = [];
  let operatorApprovals: Array<{ id: string; action_kind: string; target: string | null; state: string }> = [];
  let operatorTasks: Array<{ id: string; name: string; cron: string; enabled: boolean; next_run_at: string | null }> = [];
  let operatorTitle = 'Workspace follow-up';
  let operatorGoal = 'Summarize pending work and propose next actions';
  let operatorPlanText = 'Collect context\nDraft action proposals\nWait for approvals';
  let operatorTaskName = 'Morning research check';
  let operatorTaskCron = '0 */30 * * * * *';
  let operatorTaskPrompt = 'Check project status and generate a short briefing.';
  let info = '';

  async function invoke<T>(cmd: string, args: Record<string, unknown> = {}): Promise<T> {
    const { invoke } = await import('@tauri-apps/api/core');
    return invoke<T>(cmd, args);
  }

  async function refresh() {
    try {
      status = await invoke<Status>('get_status');
      tasks = await invoke('list_tasks');
      skills = await invoke('list_skills');
      logs = await invoke('get_audit_events', { limit: 20 });
      await refreshOperator();
      info = 'Synced from local daemon.';
    } catch (e) {
      info = `Desktop API unavailable in browser preview: ${String(e)}`;
    }
  }

  async function refreshOperator() {
    operatorSessions = await invoke('operator_list_sessions', { limit: 20 });
    operatorApprovals = await invoke('operator_pending_approvals', { limit: 50 });
    operatorTasks = await invoke('operator_list_tasks', { limit: 20 });
    if (!selectedSessionId && operatorSessions.length > 0) {
      selectedSessionId = operatorSessions[0].id;
    }
    if (selectedSessionId) {
      operatorTimeline = await invoke('operator_timeline', { session_id: selectedSessionId, limit: 100 });
    } else {
      operatorTimeline = [];
    }
  }

  async function requestElevation() {
    await invoke('request_elevation', { ttl_seconds: ttl });
    await refresh();
  }

  async function approveToken() {
    if (!token) return;
    await invoke('approve_action', { token });
    token = '';
    await refresh();
  }

  async function runSkill() {
    try {
      const input = JSON.parse(payload);
      const result = await invoke('run_skill', {
        skill_id: skillId,
        input,
        approval_token: null
      });
      info = JSON.stringify(result, null, 2);
      await refresh();
    } catch (e: any) {
      const tokenFromError = e?.confirmation_token;
      if (tokenFromError) {
        token = tokenFromError;
      }
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  async function createSkill() {
    try {
      const result = await invoke('create_skill', {
        skill_id: newSkillId,
        name: newSkillName,
        description: 'Generated from dashboard',
        approval_token: null
      });
      info = JSON.stringify(result, null, 2);
      await refresh();
    } catch (e: any) {
      const tokenFromError = e?.confirmation_token;
      if (tokenFromError) token = tokenFromError;
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  async function storeMemory() {
    try {
      const id = await invoke<string>('memory_store', {
        text: memoryText,
        embedding: null,
        approval_token: null
      });
      info = `Stored memory id: ${id}`;
      await refresh();
    } catch (e: any) {
      const tokenFromError = e?.confirmation_token;
      if (tokenFromError) token = tokenFromError;
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  async function searchMemory() {
    try {
      memoryHits = await invoke('memory_search', { query: memoryQuery, limit: 5 });
      info = `Found ${memoryHits.length} memory hit(s)`;
    } catch (e) {
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  async function loadBriefing() {
    try {
      briefing = await invoke('get_daily_briefing');
      info = 'Loaded daily briefing.';
    } catch (e) {
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  async function createOperatorSession() {
    try {
      const created = await invoke<{ id: string }>('operator_create_session', {
        title: operatorTitle,
        goal: operatorGoal
      });
      selectedSessionId = created.id;
      await refreshOperator();
      info = `Created operator session: ${created.id}`;
    } catch (e) {
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  async function planOperatorSession() {
    if (!selectedSessionId) return;
    try {
      const steps = operatorPlanText
        .split('\n')
        .map((s) => s.trim())
        .filter((s) => s.length > 0);
      await invoke('operator_plan_session', { session_id: selectedSessionId, steps });
      await refreshOperator();
      info = `Planned session: ${selectedSessionId}`;
    } catch (e) {
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  async function runOperatorSession() {
    if (!selectedSessionId) return;
    try {
      await invoke('operator_run_session', { session_id: selectedSessionId });
      await refreshOperator();
      info = `Run requested: ${selectedSessionId}`;
    } catch (e) {
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  async function approveProposal(actionId: string) {
    try {
      await invoke('operator_approve_proposal', { action_id: actionId, actor: 'dashboard_ui' });
      await refreshOperator();
      info = `Approved proposal: ${actionId}`;
    } catch (e) {
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  async function rejectProposal(actionId: string) {
    try {
      await invoke('operator_reject_proposal', { action_id: actionId, actor: 'dashboard_ui' });
      await refreshOperator();
      info = `Rejected proposal: ${actionId}`;
    } catch (e) {
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  async function createOperatorTask() {
    try {
      await invoke('operator_create_task', {
        name: operatorTaskName,
        cron: operatorTaskCron,
        prompt: operatorTaskPrompt,
        target_project: null,
        enabled: true
      });
      await refreshOperator();
      info = `Created operator task: ${operatorTaskName}`;
    } catch (e) {
      info = typeof e === 'string' ? e : JSON.stringify(e, null, 2);
    }
  }

  onMount(() => {
    refresh();
    const id = setInterval(refresh, 20000);
    return () => clearInterval(id);
  });
</script>

<main>
  <section class="hero">
    <h1>Clawork Control Deck</h1>
    <p>Sandbox-first desktop AI runtime with daemon heartbeat, cron, adapters, and WASM skills.</p>
    <div class="info">{info}</div>
  </section>

  <section class="grid">
    <article class="card">
      <h2>Status</h2>
      {#if status}
        <ul>
          <li>Daemon: {status.daemon_running ? 'running' : 'stopped'}</li>
          <li>Last heartbeat: {status.last_heartbeat ?? 'n/a'}</li>
          <li>Loaded skills: {status.loaded_skills}</li>
          <li>Adapters: {JSON.stringify(status.adapters)}</li>
          {#if status.operator}
            <li>Operator sessions: {status.operator.active_sessions}</li>
            <li>Pending approvals: {status.operator.pending_approvals}</li>
            <li>Scheduled operator tasks: {status.operator.scheduled_tasks}</li>
          {/if}
        </ul>
      {:else}
        <p>No status yet.</p>
      {/if}
    </article>

    <article class="card">
      <h2>Permissions</h2>
      <label>
        Elevated TTL (seconds)
        <input type="number" bind:value={ttl} min="30" />
      </label>
      <button on:click={requestElevation}>Request Elevated Session</button>
      <label>
        Approval token
        <input type="text" bind:value={token} placeholder="confirm-..." />
      </label>
      <button on:click={approveToken}>Approve Action Token</button>
    </article>

    <article class="card">
      <h2>Run Skill</h2>
      <label>
        Skill ID
        <input type="text" bind:value={skillId} />
      </label>
      <label>
        JSON payload
        <textarea bind:value={payload}></textarea>
      </label>
      <button on:click={runSkill}>Execute</button>
    </article>
  </section>

  <section class="grid secondary">
    <article class="card">
      <h2>Scheduled Tasks</h2>
      <ul>
        {#each tasks as task}
          <li><strong>{task.id}</strong> - {task.label}</li>
        {/each}
      </ul>
    </article>

    <article class="card">
      <h2>Skills</h2>
      <ul>
        {#each skills as skill}
          <li>{skill.id} ({skill.abi_version})</li>
        {/each}
      </ul>
    </article>

    <article class="card">
      <h2>Audit</h2>
      <ul>
        {#each logs as log}
          <li>{log.timestamp} - {log.action} - {log.decision}</li>
        {/each}
      </ul>
    </article>
  </section>

  <section class="grid secondary">
    <article class="card">
      <h2>Create Skill</h2>
      <label>
        Skill ID
        <input type="text" bind:value={newSkillId} />
      </label>
      <label>
        Skill Name
        <input type="text" bind:value={newSkillName} />
      </label>
      <button on:click={createSkill}>Generate Skill</button>
    </article>

    <article class="card">
      <h2>Memory</h2>
      <label>
        Store text
        <textarea bind:value={memoryText}></textarea>
      </label>
      <button on:click={storeMemory}>Store Memory</button>
      <label>
        Search query
        <input type="text" bind:value={memoryQuery} />
      </label>
      <button on:click={searchMemory}>Search Memory</button>
      <ul>
        {#each memoryHits as hit}
          <li>{hit.score.toFixed(3)} - {hit.record.text}</li>
        {/each}
      </ul>
    </article>

    <article class="card">
      <h2>Daily Briefing</h2>
      <button on:click={loadBriefing}>Load Briefing</button>
      {#if briefing}
        <p>{briefing.overview}</p>
        <ul>
          <li>Tasks due: {briefing.tasks_due}</li>
          <li>Recent audits: {briefing.recent_audit_events}</li>
          <li>Memory entries: {briefing.memory_entries}</li>
        </ul>
        <h3>Suggestions</h3>
        <ul>
          {#each briefing.suggestions as s}
            <li>{s.at} - {s.text}</li>
          {/each}
        </ul>
      {/if}
    </article>
  </section>

  <section class="grid secondary">
    <article class="card">
      <h2>Operator Sessions</h2>
      <label>
        Session title
        <input type="text" bind:value={operatorTitle} />
      </label>
      <label>
        Goal
        <textarea bind:value={operatorGoal}></textarea>
      </label>
      <button on:click={createOperatorSession}>Create Session</button>
      <label>
        Select session
        <select bind:value={selectedSessionId} on:change={refreshOperator}>
          <option value="">(none)</option>
          {#each operatorSessions as s}
            <option value={s.id}>{s.title} [{s.state}]</option>
          {/each}
        </select>
      </label>
      <label>
        Plan steps (one per line)
        <textarea bind:value={operatorPlanText}></textarea>
      </label>
      <button on:click={planOperatorSession}>Plan Session</button>
      <button on:click={runOperatorSession}>Run Session</button>
    </article>

    <article class="card">
      <h2>Operator Timeline</h2>
      <ul>
        {#each operatorTimeline as item}
          <li>{item.at} - {item.kind} - {item.summary}</li>
        {/each}
      </ul>
    </article>

    <article class="card">
      <h2>Approval Queue</h2>
      <ul>
        {#each operatorApprovals as action}
          <li>
            {action.action_kind} - {action.target ?? 'n/a'}
            <button on:click={() => approveProposal(action.id)}>Approve</button>
            <button on:click={() => rejectProposal(action.id)}>Reject</button>
          </li>
        {/each}
      </ul>
    </article>
  </section>

  <section class="grid secondary">
    <article class="card">
      <h2>Operator Tasks</h2>
      <label>
        Task name
        <input type="text" bind:value={operatorTaskName} />
      </label>
      <label>
        Cron
        <input type="text" bind:value={operatorTaskCron} />
      </label>
      <label>
        Prompt
        <textarea bind:value={operatorTaskPrompt}></textarea>
      </label>
      <button on:click={createOperatorTask}>Create Task</button>
      <ul>
        {#each operatorTasks as task}
          <li>{task.name} - {task.cron} - next: {task.next_run_at ?? 'n/a'}</li>
        {/each}
      </ul>
    </article>
  </section>
</main>
