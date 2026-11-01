/* Synthetic design module. No network requests or live telemetry mutations. */
(() => {
  const icons = {
    pulse: 'M2 12h4l3-8 5 16 3-8h5',
    chart: 'M4 20V10m6 10V4m6 16v-7m6 7H2',
    server: 'M4 3h16v7H4zM4 14h16v7H4zM7 6h.01M7 17h.01M11 6h6M11 17h6',
    eye: 'M2 12s4-7 10-7 10 7 10 7-4 7-10 7-10-7-10-7zM15 12a3 3 0 1 1-6 0 3 3 0 0 1 6 0',
    chip: 'M7 7h10v10H7zM9 1v4M15 1v4M9 19v4M15 19v4M1 9h4M1 15h4M19 9h4M19 15h4',
    flag: 'M5 22V3m0 1c5-4 9 4 15 0v10c-6 4-10-4-15 0',
    clock: 'M12 8v5l3 2M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0',
    arrow: 'M5 12h14M14 7l5 5-5 5',
    tokens: 'M12 2l9 5v10l-9 5-9-5V7zM3 7l9 5 9-5M12 12v10',
    shield: 'M12 2l8 4v6c0 6-8 10-8 10S4 18 4 12V6zM8 12l3 3 5-6',
    link: 'M9 15l6-6M7 16l-1 1a4 4 0 0 1-6-6l5-5a4 4 0 0 1 6 0M13 8l1-1a4 4 0 0 1 6 6l-5 5a4 4 0 0 1-6 0'
  };
  const icon = (name, size = 18) => `<svg width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="${icons[name] || icons.pulse}"/></svg>`;
  const safe = value => String(value).replace(/[&<>"']/g, character => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[character]));
  const active = () => freshRuns(inventoryRuns);
  const unknown = runs => runs.filter(run => ['Unknown', 'Stop unknown'].includes(runState(run))).length;
  const dot = (label, tone = 'mint') => `<span class="tv-status tv-${tone}"><i></i>${safe(label)}</span>`;
  const header = (title, subtitle, right = '') => `<header class="tv-card-head"><div><h2>${title}</h2><p>${subtitle}</p></div>${right}</header>`;
  const panel = (content, extraClass = '') => `<section class="tv-panel ${extraClass}">${content}</section>`;

  function ring(segments, center, sub, label) {
    const total = segments.reduce((sum, segment) => sum + segment.value, 0);
    const radius = 56;
    const circumference = 2 * Math.PI * radius;
    let offset = 0;
    const arcs = segments.filter(segment => segment.value > 0).map(segment => {
      const length = total ? segment.value / total * circumference : 0;
      const svg = `<circle cx="80" cy="80" r="${radius}" fill="none" stroke="${segment.color}" stroke-width="13" stroke-dasharray="${Math.max(0, length - 3)} ${circumference}" stroke-dashoffset="${-offset}" transform="rotate(-90 80 80)"/>`;
      offset += length;
      return svg;
    }).join('');
    return `<svg class="tv-ring" viewBox="0 0 160 160" role="img" aria-label="${safe(label)}"><circle cx="80" cy="80" r="${radius}" fill="none" stroke="var(--tv-track)" stroke-width="13"/>${arcs}<text x="80" y="79" text-anchor="middle" class="tv-ring-number">${safe(center)}</text><text x="80" y="99" text-anchor="middle" class="tv-ring-sub">${safe(sub)}</text></svg>`;
  }

  function activityPlot() {
    // Twelve illustrative five-minute event buckets. This is Alex's rule sample,
    // not a reconstruction of the snapshot inventory or a fleet-wide real series.
    const starts = [0, 1, 0, 2, 1, 0, 2, 1, 0, 2, 1, 2];
    const x = index => 43 + index * 47;
    const bar = starts.map((value, index) => `<g><title>${Math.floor((12*60+45+index*5)/60)}:${String((45+index*5)%60).padStart(2,'0')} UTC · ${value} accepted start events · synthetic bucket</title><rect x="${x(index)}" y="${173 - value * 58}" width="21" height="${value * 58}" rx="5" fill="${index > 8 ? '#7762ed' : '#c8bffa'}"/><text x="${x(index) + 10}" y="${value ? 163 - value * 58 : 164}" text-anchor="middle" class="tv-value">${value}</text></g>`).join('');
    return panel(header('Activity, with context', 'Alex Lee · synthetic start events · 12:45–13:40 UTC', dot('5-minute buckets', 'lavender')) + `<div class="tv-activity-meta"><div><strong>12</strong><span>session starts</span></div><div class="tv-baseline">${icon('pulse', 15)} Sample baseline <b>3 / hour</b></div></div><div class="tv-plot"><svg viewBox="0 0 620 225" role="img" aria-label="Twelve synthetic session start events across twelve five-minute buckets. Counts are 0,1,0,2,1,0,2,1,0,2,1,2."><g stroke="var(--tv-grid)" stroke-dasharray="3 5"><path d="M35 173H610M35 115H610M35 57H610"/></g><g class="tv-axis"><text x="16" y="177">0</text><text x="16" y="119">1</text><text x="16" y="61">2</text><text x="35" y="213">12:45</text><text x="175" y="213">13:00</text><text x="315" y="213">13:15</text><text x="455" y="213">13:30</text><text x="574" y="213">13:40</text></g>${bar}</svg></div><footer class="tv-chart-foot"><span><i class="tv-key tv-lavender"></i>Accepted start events / 5 min</span><button data-signal="sig_sessions">Review spike ${icon('arrow', 13)}</button></footer>`, 'tv-activity');
  }

  function connectionPanel() {
    const online = inventoryDevices.filter(device => device.online).length;
    const total = inventoryDevices.length;
    return panel(header('Connection coverage', 'Native host observations · current snapshot', icon('server')) + `<div class="tv-coverage-body">${ring([{value:online,color:'#2ba98d'},{value:total-online,color:'#f0b55d'}], `${online}/${total}`, 'HOSTS CURRENT', `${online} current hosts and ${total-online} offline or stale hosts`)}<div class="tv-coverage-labels"><div><span class="tv-key tv-mint"></span><b>${online}</b><span>Current</span></div><div><span class="tv-key tv-amber"></span><b>${total-online}</b><span>Offline / stale</span></div></div></div><div class="tv-coverage-note">${icon('eye', 15)}Missing observation does not mean the native process stopped.</div>`, 'tv-coverage');
  }

  function modelPanel() {
    const agents = active().filter(run => run.harness !== 'PTY shell');
    const counts = new Map();
    agents.forEach(run => counts.set(run.model || 'Not reported', (counts.get(run.model || 'Not reported') || 0) + 1));

    let knownColor = 0;
    const entries = [...counts].map(([name, value]) => ({name, value, color: name === 'Not reported' ? '#f0b55d' : ['#7762ed','#2ba98d','#63a7e7'][knownColor++ % 3]}));
    const reported = agents.filter(run => run.model).length;
    const shells = active().filter(run => run.harness === 'PTY shell').length;
    return panel(header('Harness × model', safe([...new Set(agents.map(run=>run.harness))].join(' · ') || 'No active harness') + ' · confirmed running', icon('chip')) + `<div class="tv-model-layout">${ring(entries, agents.length, 'AGENT PROCESSES', entries.map(entry=>`${entry.name}: ${entry.value}`).join(', '))}<div class="tv-model-legend">${entries.map(entry=>`<div><i style="background:${entry.color}"></i><span>${safe(entry.name)}</span><b>${entry.value}</b></div>`).join('') || '<p>No confirmed running agents.</p>'}<hr/><div><i style="background:#7e93a5"></i><span>PTY shell · no model</span><b>${shells}</b></div></div></div><footer class="tv-text-foot">Model reporting: <b>${reported} of ${agents.length}</b> active agent sessions. Unknown values stay visible.</footer>`, 'tv-model');
  }

  function ownerPlot() {
    const userCounts = inventoryUsers.map(user => {
      const devices = inventoryDevices.filter(device => device.owner === user.id);
      const runs = inventoryRuns.filter(run => devices.some(device => device.id === run.device));
      return {user, running:freshRuns(runs).length, stale:unknown(runs)};
    });
    const maximum = Math.max(1, ...userCounts.map(row => row.running + row.stale));
    return panel(header('Execution by owner', 'Process inventory · confirmed and unknown', icon('chart')) + `<div class="tv-owner-chart">${userCounts.map(({user,running,stale}) => `<div class="tv-owner-row"><button data-user="${safe(user.id)}"><span class="tv-avatar">${safe(user.initials)}</span>${safe(user.name)}</button><div class="tv-owner-track" role="img" aria-label="${safe(user.name)}: ${running} confirmed running sessions, ${stale} unknown sessions"><i style="width:${running/maximum*100}%" class="tv-segment-mint"></i><i style="width:${stale/maximum*100}%" class="tv-segment-amber"></i></div><span><b>${running}</b> <small>+${stale} ?</small></span></div>`).join('')}<div class="tv-owner-axis"><span>0</span><span>${maximum} sessions</span></div></div><footer class="tv-chart-foot"><span><i class="tv-key tv-mint"></i>Confirmed running</span><span><i class="tv-key tv-amber"></i>Unknown</span></footer>`, 'tv-owners');
  }

  function usagePanel() {
    const agents = active().filter(run=>run.harness !== 'PTY shell');
    const reported = agents.filter(run=>Number.isFinite(run.tokens));
    const tokens = reported.reduce((sum, run)=>sum+run.tokens,0);
    return panel(header('Usage reporting', 'Partial session-lifetime observation', icon('tokens')) + `<div class="tv-usage-number">${tokens.toLocaleString('en-US')}<small>reported tokens</small></div><div class="tv-usage-track"><i style="width:${agents.length?reported.length/agents.length*100:0}%"></i></div><div class="tv-usage-description"><b>${reported.length} of ${agents.length} active agent sessions</b><p>Sessions without usage are excluded. This is neither fleet spend nor a last-hour token total.</p></div><div class="tv-usage-tag">${icon('eye',14)}Partial coverage</div>`, 'tv-usage');
  }

  function signalCards() {
    const signalIcons = ['server','pulse','eye'];
    return `<section class="tv-signals"><header class="tv-section-title"><div><div class="tv-eyebrow">REVIEW, NOT AUTOMATIC ENFORCEMENT</div><h2>Signals worth a closer look<span>${unusualSignals.length}</span></h2></div><span class="tv-small">Synthetic rules · eval_demo_13</span></header><div class="tv-signal-grid">${unusualSignals.map((signal,index)=>`<article class="tv-signal-card searchable tv-signal-${index}"><div class="tv-signal-top"><span class="tv-signal-icon">${icon(signalIcons[index],20)}</span>${dot(signal.severity, index===2?'sky':'amber')}</div><div class="tv-eyebrow">${safe(signal.who)}</div><h3>${safe(signal.title)}</h3><p>${safe(signal.rule)}</p><details><summary>Evidence & context</summary><p>${safe(signal.evidence)}</p><p>${safe(signal.context)}</p></details><button data-signal="${safe(signal.id)}">Inspect evidence ${icon('arrow',14)}</button></article>`).join('')}</div></section>`;
  }

  function render() {
    const runs=active();
    const agents=runs.filter(run=>run.harness!=='PTY shell');
    const online=inventoryDevices.filter(device=>device.online).length;
    const stale=unknown(inventoryRuns);
    return `<div class="tv-dashboard"><div class="tv-page-top"><div><div class="tv-eyebrow">ADMINISTRATION / TELEMETRY</div><h1>A pulse on every connection.</h1><p>Know what is running, what is changing, and where the picture is incomplete.</p></div><div class="tv-snapshot">${icon('clock',14)}<span>Sample snapshot<br><b>13:42:16 UTC</b></span></div></div><div class="tv-scope-ribbon"><span>${icon('layers',14)}Synthetic cohort: ${inventoryUsers.length} users · ${inventoryDevices.length} devices</span><span>${icon('shield',14)}Metadata only. No transcript content.</span></div><div class="tv-stat-grid">${[
      ['server','mint','Hosts reporting',`${online}<em> / ${inventoryDevices.length}</em>`,'Connected / enrolled hosts'],
      ['pulse','lavender','Native sessions',runs.length,`${stale} unknown sessions excluded`],
      ['chip','sky','Agent processes',agents.length,'Installed harnesses are not executions'],
      ['flag','amber','Review signals',unusualSignals.length,'No automatic stop or revocation']
    ].map(([i,t,label,value,note])=>`<div class="tv-stat tv-${t}"><div class="tv-stat-icon">${icon(i,19)}</div><div><label>${label}</label><strong>${value}</strong><small>${note}</small></div></div>`).join('')}</div><div class="tv-top-grid">${activityPlot()}${connectionPanel()}</div><div class="tv-mid-grid">${modelPanel()}${ownerPlot()}${usagePanel()}</div>${signalCards()}<footer class="tv-final-note">${icon('eye',14)}All plots use synthetic data. Baselines need sufficient history; partial coverage must not be treated as zero activity.</footer></div>`;
  }
  window.TelemetryVisual = {render};
})();
