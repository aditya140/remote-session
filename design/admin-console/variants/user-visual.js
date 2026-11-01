/* Owner-only synthetic workspace. Read source state on every render. */
(() => {
  const colors = { blue: '#3975f6', teal: '#00a895', amber: '#efaa32', pink: '#dd5b91', slate: '#8a96aa' };
  const escape = value => String(value ?? '').replace(/[&<>"']/g, char => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[char]));
  const icon = (name, size = 20) => {
    const paths = {
      shield: '<path d="M16 3l11 4v9c0 8-11 13-11 13S5 24 5 16V7zM10 16l4 4 8-9"/>',
      layers: '<path d="M16 2L2 9l14 7 14-7zM2 16l14 7 14-7M2 23l14 7 14-7"/>',
      laptop: '<rect x="4" y="3" width="24" height="18" rx="2"/><path d="M2 25h28l-3 3H5zM12 25h8"/>',
      server: '<rect x="5" y="3" width="22" height="11" rx="3"/><rect x="5" y="18" width="22" height="11" rx="3"/><path d="M10 8h.1M10 23h.1M16 8h6M16 23h6"/>',
      agent: '<rect x="5" y="8" width="22" height="20" rx="6"/><path d="M16 3v5M10 15v3M22 15v3M11 23h10M2 15v7M30 15v7"/><circle cx="16" cy="3" r="1"/>',
      shell: '<rect x="3" y="5" width="26" height="22" rx="4"/><path d="M9 12l5 4-5 4M18 21h6"/>',
      pulse: '<path d="M2 17h6l4-10 7 20 4-10h7"/>',
      stop: '<rect x="6" y="6" width="20" height="20" rx="5"/><rect x="12" y="12" width="8" height="8" rx="1" fill="currentColor" stroke="none"/>',
      link: '<path d="M12 21l-2 2a6 6 0 0 1-8-8l6-6a6 6 0 0 1 8 0M20 11l2-2a6 6 0 0 1 8 8l-6 6a6 6 0 0 1-8 0M10 22l12-12"/>',
      arrow: '<path d="M6 16h20M19 9l7 7-7 7"/>',
      clock: '<circle cx="16" cy="16" r="12"/><path d="M16 8v9l6 3"/>',
      check: '<circle cx="16" cy="16" r="12"/><path d="M10 16l4 4 8-9"/>',
      cloud: '<path d="M8 25a7 7 0 0 1-1-14 10 10 0 0 1 19-1 8 8 0 0 1-1 15H8z"/>',
      model: '<path d="M16 2l12 7v14l-12 7-12-7V9zM4 9l12 7 12-7M16 16v14"/>',
      warning: '<path d="M14 4a2 2 0 0 1 4 0l13 23H1zM16 11v8M16 24h.1"/>',
      refresh: '<path d="M27 13a11 11 0 0 0-19-5L4 12M4 4v8h8M5 19a11 11 0 0 0 19 5l4-4M28 28v-8h-8"/>',
    };
    return `<svg width="${size}" height="${size}" viewBox="0 0 32 32" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] || paths.pulse}</svg>`;
  };
  const badge = (text, kind = 'blue') => `<span class="uv-badge uv-${kind}"><i></i>${escape(text)}</span>`;
  function donut(segments, total, middle, description) {
    let offset = 0;
    const rings = segments.filter(segment => segment.value > 0).map(segment => {
      const length = total ? segment.value / total * 100 : 0;
      const ring = `<circle cx="64" cy="64" r="48" pathLength="100" fill="none" stroke="${colors[segment.color]}" stroke-width="12" stroke-dasharray="${Math.max(0, length - 1.5)} ${100 - Math.max(0, length - 1.5)}" stroke-dashoffset="${-offset}"/>`;
      offset += length;
      return ring;
    }).join('');
    return `<svg viewBox="0 0 128 128" class="uv-donut" role="img" aria-label="${escape(description)}"><circle cx="64" cy="64" r="48" fill="none" stroke="var(--uv-track)" stroke-width="12"/><g transform="rotate(-90 64 64)">${rings}</g><text x="64" y="63" text-anchor="middle" class="uv-donut-value">${escape(middle)}</text><text x="64" y="80" text-anchor="middle" class="uv-donut-label">${escape(description)}</text></svg>`;
  }
  function deviceCard(device, index) {
    const runs = deviceRuns(device.id), active = freshRuns(runs);
    const unknown = runs.filter(run => ['Unknown', 'Stop unknown'].includes(runState(run))).length;
    const tone = ['blue', 'teal', 'amber'][index % 3];
    return `<article class="uv-device uv-device-${tone} searchable"><div class="uv-device-top"><span class="uv-device-icon uv-${tone}">${icon(device.kind === 'Server' ? 'server' : 'laptop', 28)}</span>${badge(device.online ? 'Connected' : 'Offline', device.online ? 'teal' : 'amber')}</div><h3>${escape(device.name)}</h3><p>${escape(device.os)} <span> / </span> ${escape(device.kind)}</p><div class="uv-device-count"><strong>${active.length}</strong><span>confirmed running sessions${unknown ? `<small>${unknown} current state unknown</small>` : '<small>Fresh native observation</small>'}</span></div><div class="uv-device-rail">${runs.map(run => `<span class="${['Unknown', 'Stop unknown'].includes(runState(run)) ? 'uv-amber' : runState(run) === 'Exited' ? 'uv-slate' : tone}">${icon(run.harness === 'PTY shell' ? 'shell' : 'agent', 14)}${escape(run.harness)}</span>`).join('')}</div><div class="uv-device-detail"><span>${icon('clock', 13)}Last seen</span><time datetime="${escape(device.seenAt)}">${escape(device.lastSeen)}</time></div><div class="uv-device-detail"><span>Last connected</span><span>${escape(device.lastConnected)}</span></div><button class="uv-device-open" data-device="${escape(device.id)}">View sessions ${icon('arrow', 15)}</button></article>`;
  }
  function sessionCard(run) {
    const device = inventoryDevices.find(item => item.id === run.device), state = runState(run);
    const isShell = run.harness === 'PTY shell';
    const tone = state === 'Exited' ? 'slate' : ['Unknown', 'Stop unknown'].includes(state) ? 'amber' : isShell ? 'teal' : 'blue';
    const taskTone = state === 'Exited' ? 'slate' : run.task === 'Turn failed' ? 'pink' : run.task === 'Unknown' ? 'amber' : 'teal';
    let action;
    if (state === 'Stop unknown') action = `<button class="uv-reconcile" data-reconcile="${escape(run.id)}">${icon('refresh', 15)} Check outcome</button>`;
    else action = `<button class="uv-stop" data-stop="${escape(run.id)}" ${state !== 'Running' || !device.online ? 'disabled' : ''}>${icon('stop', 15)}${state === 'Exited' ? 'Stopped' : !device.online ? 'Host offline' : 'Stop session'}</button>`;
    return `<article class="uv-session searchable" data-session-row="${escape(run.id)}"><div class="uv-session-main"><span class="uv-session-icon uv-${tone}">${icon(isShell ? 'shell' : 'agent', 24)}</span><div class="uv-session-title"><h3>${escape(run.name)}</h3><p><span>${icon(device.kind === 'Server' ? 'server' : 'laptop', 13)}${escape(device.name)}</span><span>${escape(run.harness)} ${escape(run.version)}</span></p></div>${badge(state, tone)}</div><div class="uv-session-meta"><div><span>MODEL</span><b>${isShell ? 'Not applicable' : escape(run.model || 'Not reported')}</b><small>${isShell ? 'Terminal session' : escape(run.source)}</small></div><div><span>TASK STATE</span>${badge(state === 'Exited' ? 'Session stopped' : run.task, taskTone)}<small>Separate from native process state</small></div><div><span>STARTED</span><b>${escape(run.started)}</b><small>${escape(run.id)}</small></div></div><div class="uv-session-actions"><span>${icon('link', 13)}${device.online ? 'Current host connection' : 'Last-known native session'}</span>${action}</div></article>`;
  }
  function render() {
    const devices = ownedDevices('alex'), runs = userRuns('alex'), active = freshRuns(runs);
    const connected = devices.filter(device => device.online).length;
    const unknown = runs.filter(run => ['Unknown', 'Stop unknown'].includes(runState(run))).length;
    const exited = runs.filter(run => runState(run) === 'Exited').length;
    const activeAgents = active.filter(run => run.harness !== 'PTY shell');
    const knownModels = activeAgents.filter(run => Boolean(run.model)).length;
    const taskFailures = active.filter(run => run.task === 'Turn failed').length;
    const modeledUsage = activeAgents.filter(run => Number.isFinite(run.tokens));
    const usage = modeledUsage.reduce((sum, run) => sum + run.tokens, 0);
    const maxSessions = Math.max(1, ...devices.map(device => freshRuns(deviceRuns(device.id)).length));
    return `<div class="uv-root"><header class="uv-heading"><div><div class="uv-kicker">${icon('layers', 15)} YOUR WORKSPACE / DEVICE CONTROL</div><h1>Your agents. Your devices.<br><em>One clear picture.</em></h1><p>Know where work is happening, which model is reported, and what needs your attention.</p></div><div class="uv-owner"><span class="uv-owner-avatar">AL</span><div><strong>Alex Lee</strong><small>Owner scope only · synthetic data</small></div></div></header><div class="uv-summary"><div class="uv-summary-item"><span class="uv-summary-icon uv-blue">${icon('laptop', 22)}</span><div><strong>${devices.length}<small>devices</small></strong><span>${connected} connected · ${devices.length - connected} offline</span></div></div><div class="uv-summary-item"><span class="uv-summary-icon uv-teal">${icon('pulse', 23)}</span><div><strong>${active.length}<small>active sessions</small></strong><span>${unknown} unknown · ${exited} stopped</span></div></div><div class="uv-summary-item"><span class="uv-summary-icon uv-pink">${icon('agent', 23)}</span><div><strong>${activeAgents.length}<small>running agents</small></strong><span>${taskFailures} failed turn${taskFailures === 1 ? '' : 's'} in a running process</span></div></div><div class="uv-summary-item"><span class="uv-summary-icon uv-amber">${icon('server', 22)}</span><div><strong>${devices.filter(device => device.kind === 'Server').length}<small>servers</small></strong><span>A subset of enrolled devices</span></div></div></div><div class="uv-insight-grid"><section class="uv-panel uv-coverage"><div class="uv-panel-title"><h2>${icon('cloud', 17)} Connection coverage</h2><span>SNAPSHOT</span></div><div class="uv-chart-body">${donut([{ value: connected, color: 'blue' }, { value: devices.length - connected, color: 'amber' }], devices.length, `${connected}/${devices.length}`, 'devices online')}<div class="uv-legend"><div><i class="uv-dot-blue"></i><b>${connected}</b> Connected</div><div><i class="uv-dot-amber"></i><b>${devices.length - connected}</b> Offline</div><p>Offline does not mean native work has stopped.</p></div></div></section><section class="uv-panel uv-workload"><div class="uv-panel-title"><h2>${icon('pulse', 17)} Where sessions run</h2><span>CONFIRMED ACTIVE</span></div><div class="uv-workload-chart" role="img" aria-label="Confirmed active sessions by your device; unknown observations are separate">${devices.map((device, index) => { const count = freshRuns(deviceRuns(device.id)).length; const missing = deviceRuns(device.id).filter(run => ['Unknown', 'Stop unknown'].includes(runState(run))).length; return `<div class="uv-workload-row"><div><span>${icon(device.kind === 'Server' ? 'server' : 'laptop', 14)}${escape(device.name)}</span><b>${count}${missing ? `<small> + ${missing} unknown</small>` : ''}</b></div><div class="uv-bar"><i class="uv-bar-${['blue', 'teal', 'amber'][index]}" style="width:${100 * count / maxSessions}%"></i></div></div>`; }).join('')}<div class="uv-axis"><span>0</span><span>${maxSessions} sessions</span></div></div></section><section class="uv-panel uv-models"><div class="uv-panel-title"><h2>${icon('model', 17)} Model visibility</h2><span>ACTIVE AGENTS</span></div><div class="uv-chart-body">${donut([{ value: knownModels, color: 'pink' }, { value: activeAgents.length - knownModels, color: 'slate' }], activeAgents.length, `${knownModels}/${activeAgents.length}`, 'models reported')}<div class="uv-legend"><div><i class="uv-dot-pink"></i><b>${knownModels}</b> Reported model</div><div><i class="uv-dot-slate"></i><b>${activeAgents.length - knownModels}</b> Not reported</div><p>Terminal sessions are excluded. No model is inferred.</p></div></div></section></div><div class="uv-section-title"><div><span class="uv-kicker">YOUR ENROLLED HOSTS</span><h2>A home for every session.</h2></div><span class="uv-small-label">${icon('check', 14)} Last accepted observations shown per device</span></div><div class="uv-device-grid">${devices.map(deviceCard).join('')}</div><div class="uv-section-title uv-session-heading"><div><span class="uv-kicker">NATIVE EXECUTION</span><h2>Agents and terminal sessions <span>${runs.length}</span></h2></div><span class="uv-small-label">${icon('stop', 14)} Stop controls target one session</span></div><div class="uv-session-layout"><div class="uv-sessions">${runs.map(sessionCard).join('')}</div><div class="uv-context"><section class="uv-usage-card"><span class="uv-summary-icon uv-pink">${icon('model', 23)}</span><div class="uv-kicker">REPORTED TOKEN USAGE</div><strong>${usage.toLocaleString('en-US')}</strong><p>Partial session-lifetime total</p><div class="uv-usage-meter"><i style="width:${activeAgents.length ? modeledUsage.length / activeAgents.length * 100 : 0}%"></i></div><small>${modeledUsage.length} of ${activeAgents.length} active agents report usage. This is not total spend.</small></section><section class="uv-permission-note"><span class="uv-summary-icon uv-amber">${icon('shield', 22)}</span><h3>Stop is a separate decision.</h3><p>Stopping ends the selected native session after confirmation. Other sessions and device access continue.</p><p>A missing reply remains unknown until reconciled. No automatic resend.</p></section><div class="uv-source-note">${icon('clock', 14)} Synthetic snapshot<br>Oct 31 · 13:42:16 UTC<br>All actions are local simulations.</div></div></div></div>`;
  }
  window.UserVisual = { render };
})();
