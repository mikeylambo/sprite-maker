<script lang="ts">
  import { Crosshair, Plus, Save, Square, Trash2, Wand2 } from "lucide-svelte";
  import { api, assetUrl } from "$lib/api";
  import { errorMessage, type Asset, type AssetProduction, type HitRegion, type SocketPoint } from "$lib/types";

  let { asset, onError, onNotice, onPrepared }: {
    asset: Asset;
    onError: (message: string) => void;
    onNotice: (message: string) => void;
    onPrepared: (prepared: Asset) => void | Promise<void>;
  } = $props();

  let loadedAssetId = $state("");
  let sockets = $state<SocketPoint[]>([]);
  let hitboxes = $state<HitRegion[]>([]);
  let tagText = $state("");
  let selectedSocket = $state<number | undefined>();
  let dirty = $state(false);
  let saving = $state(false);
  let preparing = $state(false);
  let dragging: number | undefined;

  const stageWidth = 228;
  const scale = $derived(asset.width > 0 ? stageWidth / asset.width : 1);
  const stageHeight = $derived(Math.round(asset.height * scale));

  $effect(() => {
    if (asset.id === loadedAssetId) return;
    loadedAssetId = asset.id;
    selectedSocket = undefined;
    dirty = false;
    api.getAssetProduction(asset.id)
      .then((production) => { sockets = production.sockets; hitboxes = production.hitboxes; tagText = production.tags.join(", "); })
      .catch((error) => onError(errorMessage(error)));
  });

  function stagePoint(event: PointerEvent) {
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    return {
      x: Math.round(Math.max(0, Math.min(asset.width, (event.clientX - rect.left) / scale))),
      y: Math.round(Math.max(0, Math.min(asset.height, (event.clientY - rect.top) / scale))),
    };
  }

  function onStageDown(event: PointerEvent) {
    if (selectedSocket === undefined || !sockets[selectedSocket]) return;
    const { x, y } = stagePoint(event);
    sockets[selectedSocket] = { ...sockets[selectedSocket], x, y };
    dragging = selectedSocket;
    dirty = true;
    (event.currentTarget as HTMLElement).setPointerCapture?.(event.pointerId);
  }
  function onStageMove(event: PointerEvent) {
    if (dragging === undefined || !sockets[dragging]) return;
    const { x, y } = stagePoint(event);
    sockets[dragging] = { ...sockets[dragging], x, y };
    dirty = true;
  }
  const endDrag = () => { dragging = undefined; };

  function addSocket() {
    sockets = [...sockets, { name: `socket_${sockets.length + 1}`, x: Math.round(asset.width / 2), y: Math.round(asset.height / 2) }];
    selectedSocket = sockets.length - 1;
    dirty = true;
  }
  function removeSocket(index: number) {
    sockets = sockets.filter((_, position) => position !== index);
    if (selectedSocket === index) selectedSocket = undefined;
    dirty = true;
  }
  function addHitbox() {
    hitboxes = [...hitboxes, { name: `region_${hitboxes.length + 1}`, kind: "hurtbox", x: 0, y: 0, width: Math.max(1, Math.round(asset.width / 2)), height: Math.max(1, Math.round(asset.height / 2)) }];
    dirty = true;
  }
  function removeHitbox(index: number) {
    hitboxes = hitboxes.filter((_, position) => position !== index);
    dirty = true;
  }

  async function save() {
    saving = true;
    try {
      const production: AssetProduction = {
        sockets: sockets.map((socket) => ({ ...socket, name: socket.name.trim() })),
        hitboxes: hitboxes.map((region) => ({ ...region, name: region.name.trim() })),
        events: [],
        tags: tagText.split(",").map((tag) => tag.trim()).filter(Boolean),
      };
      await api.setAssetProduction(asset.id, production);
      dirty = false;
      onNotice("Production metadata saved");
    } catch (error) { onError(errorMessage(error)); } finally { saving = false; }
  }

  async function prepare() {
    preparing = true;
    try {
      const prepared = await api.prepareAssetForRigging(asset.id);
      onNotice(prepared.notes.join(" · "));
      await onPrepared(prepared.asset);
    } catch (error) { onError(errorMessage(error)); } finally { preparing = false; }
  }
</script>

<section class="production">
  <h3>PRODUCTION</h3>
  <button class="wide prepare" onclick={prepare} disabled={preparing}><Wand2 size={12}/>{preparing ? "Preparing…" : "Prepare for rigging"}</button>
  <p class="hint">Trims, centers on the profile's base unit, and seeds sockets. Writes a new prepared asset.</p>

  <div class="stage" style={`width:${stageWidth}px;height:${stageHeight}px`}
       onpointerdown={onStageDown} onpointermove={onStageMove} onpointerup={endDrag} onpointercancel={endDrag} role="presentation">
    <img src={assetUrl(asset.path)} alt={asset.name} draggable="false"/>
    <svg viewBox={`0 0 ${asset.width} ${asset.height}`} aria-hidden="true">
      {#each hitboxes as region}
        <rect class="region {region.kind}" x={region.x} y={region.y} width={region.width} height={region.height}/>
      {/each}
      {#each sockets as socket, index}
        <circle class="socket" class:selected={selectedSocket === index} cx={socket.x} cy={socket.y} r={Math.max(1.5, asset.width / 40)}/>
      {/each}
    </svg>
  </div>
  <p class="hint">{selectedSocket === undefined ? "Select a socket below, then click the image to place it." : `Click or drag on the image to move “${sockets[selectedSocket]?.name ?? ""}”.`}</p>

  <div class="row-head"><span><Crosshair size={11}/> Sockets</span><button onclick={addSocket} title="Add socket"><Plus size={12}/></button></div>
  {#each sockets as socket, index}
    <div class="entry" class:active={selectedSocket === index}>
      <button class="pick" onclick={() => selectedSocket = selectedSocket === index ? undefined : index} title="Select for placement"><Crosshair size={11}/></button>
      <input aria-label="Socket name" value={socket.name} oninput={(event) => { sockets[index] = { ...socket, name: event.currentTarget.value }; dirty = true; }}/>
      <input aria-label="Socket x" type="number" value={socket.x} oninput={(event) => { sockets[index] = { ...socket, x: Number(event.currentTarget.value) }; dirty = true; }}/>
      <input aria-label="Socket y" type="number" value={socket.y} oninput={(event) => { sockets[index] = { ...socket, y: Number(event.currentTarget.value) }; dirty = true; }}/>
      <button class="kill" onclick={() => removeSocket(index)} title="Remove socket"><Trash2 size={11}/></button>
    </div>
  {:else}
    <p class="muted">No sockets yet.</p>
  {/each}

  <div class="row-head"><span><Square size={11}/> Hit regions</span><button onclick={addHitbox} title="Add region"><Plus size={12}/></button></div>
  {#each hitboxes as region, index}
    <div class="entry region-entry">
      <input aria-label="Region name" value={region.name} oninput={(event) => { hitboxes[index] = { ...region, name: event.currentTarget.value }; dirty = true; }}/>
      <select aria-label="Region kind" value={region.kind} onchange={(event) => { hitboxes[index] = { ...region, kind: event.currentTarget.value as HitRegion["kind"] }; dirty = true; }}>
        <option value="hurtbox">hurt</option><option value="hitbox">hit</option><option value="collision">coll</option>
      </select>
      <button class="kill" onclick={() => removeHitbox(index)} title="Remove region"><Trash2 size={11}/></button>
      <div class="quad">
        <input aria-label="Region x" type="number" value={region.x} oninput={(event) => { hitboxes[index] = { ...region, x: Number(event.currentTarget.value) }; dirty = true; }}/>
        <input aria-label="Region y" type="number" value={region.y} oninput={(event) => { hitboxes[index] = { ...region, y: Number(event.currentTarget.value) }; dirty = true; }}/>
        <input aria-label="Region width" type="number" value={region.width} oninput={(event) => { hitboxes[index] = { ...region, width: Number(event.currentTarget.value) }; dirty = true; }}/>
        <input aria-label="Region height" type="number" value={region.height} oninput={(event) => { hitboxes[index] = { ...region, height: Number(event.currentTarget.value) }; dirty = true; }}/>
      </div>
    </div>
  {:else}
    <p class="muted">No hit regions yet.</p>
  {/each}

  <label class="tags">Tags<input value={tagText} oninput={(event) => { tagText = event.currentTarget.value; dirty = true; }} placeholder="melee, boss"/></label>
  <button class="wide save" onclick={save} disabled={!dirty || saving}><Save size={12}/>{saving ? "Saving…" : dirty ? "Save metadata" : "Saved"}</button>
</section>

<style>
  .production h3{font-size:9px;letter-spacing:.12em;color:var(--faint);margin:0 0 8px}
  .wide{width:100%;height:27px;border:1px solid var(--border);border-radius:5px;background:var(--surface);color:var(--muted);display:flex;align-items:center;justify-content:center;gap:6px;font:inherit;font-size:11px;cursor:pointer}
  .wide:disabled{opacity:.5;cursor:not-allowed}
  .prepare{border-color:var(--accent);color:var(--accent)}
  .save{background:var(--text);color:var(--bg);border-color:var(--text);margin-top:10px}
  .hint{font-size:9px;line-height:1.45;color:var(--faint);margin:6px 0 10px}
  .stage{position:relative;background-color:var(--preview);background-image:linear-gradient(45deg,var(--checker) 25%,transparent 25%),linear-gradient(-45deg,var(--checker) 25%,transparent 25%),linear-gradient(45deg,transparent 75%,var(--checker) 75%),linear-gradient(-45deg,transparent 75%,var(--checker) 75%);background-size:12px 12px;background-position:0 0,0 6px,6px -6px,-6px 0;border:1px solid var(--border-strong);touch-action:none;cursor:crosshair}
  .stage img{position:absolute;inset:0;width:100%;height:100%;object-fit:fill;image-rendering:pixelated;pointer-events:none}
  .stage svg{position:absolute;inset:0;width:100%;height:100%;pointer-events:none;overflow:visible}
  .stage .socket{fill:#69d2c5;stroke:#10110f;stroke-width:.6}
  .stage .socket.selected{fill:#e0a458;stroke:#fff;stroke-width:1}
  .stage .region{fill:#69d2c51a;stroke:#69d2c580;stroke-width:.6}
  .stage .region.hitbox{fill:#cc7a741a;stroke:#cc7a7480}
  .stage .region.collision{fill:#8a95a51a;stroke:#8a95a580}
  .row-head{display:flex;align-items:center;justify-content:space-between;margin:12px 0 6px}
  .row-head span{display:flex;align-items:center;gap:5px;font-size:9px;letter-spacing:.1em;color:var(--faint)}
  .row-head button,.entry .kill,.entry .pick{border:0;background:transparent;color:var(--faint);width:20px;height:20px;display:grid;place-items:center;border-radius:4px;cursor:pointer}
  .row-head button:hover,.entry .kill:hover,.entry .pick:hover{background:var(--surface-hover);color:var(--text)}
  .entry{display:grid;grid-template-columns:20px minmax(0,1fr) 40px 40px 20px;gap:3px;align-items:center;margin-bottom:4px}
  .entry.active{background:var(--selected);border-radius:4px}
  .entry.region-entry{grid-template-columns:minmax(0,1fr) 52px 20px}
  .entry input,.entry select,.tags input{height:23px;box-sizing:border-box;border:1px solid var(--border);border-radius:4px;background:var(--bg);color:var(--text);font:inherit;font-size:10px;padding:0 5px;min-width:0;width:100%}
  .entry .quad{grid-column:1/-1;display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:3px;margin-bottom:6px}
  .tags{display:block;font-size:9px;letter-spacing:.1em;color:var(--faint);margin-top:12px}
  .tags input{margin-top:5px}
  .muted{font-size:10px;color:var(--faint);margin:0 0 4px}
</style>
