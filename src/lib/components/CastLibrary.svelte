<script lang="ts">
  import { ClipboardCopy, Plus, Trash2, Users, X } from "lucide-svelte";
  import { api, assetUrl } from "$lib/api";
  import { errorMessage, IDENTITY_IMAGE_KINDS, type Identity } from "$lib/types";

  let { identities, onChange, onError, onNotice, onUseBrief }: {
    identities: Identity[];
    onChange: (identities: Identity[]) => void;
    onError: (message: string) => void;
    onNotice: (message: string) => void;
    onUseBrief: (brief: string) => void;
  } = $props();

  let selectedId = $state("");
  let editing = $state(false);
  let saving = $state(false);
  let name = $state("");
  let summary = $state("");
  let proportions = $state("");
  let scalePx = $state<number | undefined>();
  let palette = $state("");
  let forbidden = $state("");
  let vocabulary = $state("");
  let tags = $state("");

  const selected = $derived(identities.find((identity) => identity.id === selectedId));
  const splitList = (value: string) => value.split(",").map((item) => item.trim()).filter(Boolean);
  const splitLines = (value: string) => value.split("\n").map((item) => item.trim()).filter(Boolean);

  function startNew() {
    editing = true; selectedId = "";
    name = ""; summary = ""; proportions = ""; scalePx = undefined;
    palette = ""; forbidden = ""; vocabulary = ""; tags = "";
  }
  function startEdit(identity: Identity) {
    editing = true; selectedId = identity.id;
    name = identity.name; summary = identity.summary; proportions = identity.proportions;
    scalePx = identity.scalePx; palette = identity.palette.join(", ");
    forbidden = identity.forbidden.join("\n"); vocabulary = identity.vocabulary.join(", ");
    tags = identity.tags.join(", ");
  }

  async function save() {
    saving = true;
    try {
      const saved = await api.saveIdentity({
        id: selectedId || undefined,
        name,
        summary,
        proportions,
        scalePx: scalePx && scalePx > 0 ? Math.round(scalePx) : undefined,
        palette: splitList(palette),
        forbidden: splitLines(forbidden),
        vocabulary: splitList(vocabulary),
        tags: splitList(tags),
      });
      onChange(await api.listIdentities());
      selectedId = saved.id;
      editing = false;
      onNotice(`Saved ${saved.name} to the cast`);
    } catch (error) { onError(errorMessage(error)); } finally { saving = false; }
  }

  async function remove(identity: Identity) {
    if (!window.confirm(`Delete ${identity.name} and its stored reference images?`)) return;
    try {
      await api.deleteIdentity(identity.id);
      onChange(await api.listIdentities());
      if (selectedId === identity.id) { selectedId = ""; editing = false; }
    } catch (error) { onError(errorMessage(error)); }
  }

  async function removeImage(imageId: string) {
    try { await api.deleteIdentityImage(imageId); onChange(await api.listIdentities()); }
    catch (error) { onError(errorMessage(error)); }
  }

  async function copyBrief(identity: Identity) {
    try {
      const brief = await api.getIdentityBrief(identity.id);
      onUseBrief(brief);
      onNotice(`${identity.name}'s brief is in the chat composer`);
    } catch (error) { onError(errorMessage(error)); }
  }
</script>

<div class="cast">
  <header>
    <div><h1>Cast</h1><p>Persistent character identities. Anything pinned here stays consistent across chats, asset types, and sessions.</p></div>
    <button class="primary" onclick={startNew}><Plus size={13}/>New identity</button>
  </header>
  <div class="body">
    <nav>
      {#each identities as identity}
        <button class:active={selectedId === identity.id} onclick={() => startEdit(identity)}>
          <strong>{identity.name}</strong>
          <span>{identity.images.length} image{identity.images.length === 1 ? "" : "s"}{identity.scalePx ? ` · ${identity.scalePx}px` : ""}</span>
        </button>
      {:else}
        <p class="empty-nav">No identities yet.</p>
      {/each}
    </nav>
    <div class="detail">
      {#if editing}
        <form onsubmit={(event) => { event.preventDefault(); save(); }}>
          <div class="form-head"><h2>{selectedId ? "Edit identity" : "New identity"}</h2><button type="button" class="icon" aria-label="Close editor" onclick={() => editing = false}><X size={15}/></button></div>
          <label>Name<input required maxlength="120" bind:value={name} placeholder="Jo"/></label>
          <label>Summary<textarea rows="3" bind:value={summary} placeholder="Scrappy courier in a patched jacket, always mid-stride."></textarea></label>
          <label>Proportions<textarea rows="2" bind:value={proportions} placeholder="5 heads tall, oversized boots, narrow shoulders."></textarea></label>
          <div class="pair">
            <label>Sprite scale (px)<input type="number" min="1" max="4096" bind:value={scalePx} placeholder="64"/></label>
            <label>Animation vocabulary<input bind:value={vocabulary} placeholder="idle, run, hurt"/></label>
          </div>
          <label>Locked palette<input bind:value={palette} placeholder="#5b3a8e, #e0a458"/></label>
          <label>Never change (one per line)<textarea rows="3" bind:value={forbidden} placeholder={"scar on left cheek\njacket patch colors"}></textarea></label>
          <label>Tags<input bind:value={tags} placeholder="protagonist, greedrun"/></label>
          <div class="form-actions">
            {#if selected}<button type="button" class="danger" onclick={() => remove(selected)}><Trash2 size={12}/>Delete</button>{/if}
            <button type="submit" class="primary" disabled={saving}>{saving ? "Saving…" : "Save identity"}</button>
          </div>
        </form>
      {:else if selected}
        <div class="view">
          <div class="view-head">
            <div><h2>{selected.name}</h2>{#if selected.summary}<p>{selected.summary}</p>{/if}</div>
            <div class="view-actions"><button onclick={() => copyBrief(selected)}><ClipboardCopy size={12}/>Use brief in chat</button><button onclick={() => startEdit(selected)}>Edit</button></div>
          </div>
          {#if selected.images.length}
            <div class="gallery">{#each selected.images as image}<figure><img src={assetUrl(image.path)} alt={image.label}/><figcaption><span>{image.kind}</span><button aria-label="Remove image" onclick={() => removeImage(image.id)}><Trash2 size={10}/></button></figcaption></figure>{/each}</div>
          {:else}
            <p class="muted">No canonical images yet. Open a sprite in the Sprites tab and use “Add as canonical” in its inspector.</p>
          {/if}
          <dl>
            {#if selected.proportions}<div><dt>Proportions</dt><dd>{selected.proportions}</dd></div>{/if}
            {#if selected.scalePx}<div><dt>Scale</dt><dd>{selected.scalePx}px</dd></div>{/if}
            {#if selected.palette.length}<div><dt>Palette</dt><dd class="swatches">{#each selected.palette as color}<i style={`background:${color}`} title={color}></i>{/each}</dd></div>{/if}
            {#if selected.vocabulary.length}<div><dt>Animations</dt><dd>{selected.vocabulary.join(", ")}</dd></div>{/if}
            {#if selected.forbidden.length}<div><dt>Never change</dt><dd><ul>{#each selected.forbidden as rule}<li>{rule}</li>{/each}</ul></dd></div>{/if}
            {#if selected.tags.length}<div><dt>Tags</dt><dd>{selected.tags.join(", ")}</dd></div>{/if}
          </dl>
        </div>
      {:else}
        <div class="empty"><Users size={26}/><strong>Keep a character consistent</strong><p>An identity records the summary, proportions, palette, and the rules that must never drift — then feeds them into new generations as a brief.</p><button class="primary" onclick={startNew}><Plus size={13}/>New identity</button></div>
      {/if}
    </div>
  </div>
</div>

<style>
  .cast{height:100%;display:flex;flex-direction:column;background:var(--bg);min-width:0}
  header{height:58px;min-height:58px;border-bottom:1px solid var(--border);display:flex;align-items:center;justify-content:space-between;padding:0 16px}
  h1{font-size:14px;margin:0}header p{font-size:10px;color:var(--faint);margin:4px 0 0;max-width:560px}
  .primary{height:29px;border:1px solid var(--text);background:var(--text);color:var(--bg);border-radius:6px;padding:0 10px;display:flex;align-items:center;gap:6px;font:inherit;font-size:11px;cursor:pointer}
  .primary:disabled{opacity:.5;cursor:wait}
  .body{flex:1;min-height:0;display:grid;grid-template-columns:236px minmax(0,1fr)}
  nav{overflow:auto;border-right:1px solid var(--border);background:var(--sidebar);padding:8px}
  nav button{width:100%;border:1px solid transparent;border-radius:5px;background:transparent;color:var(--text);padding:8px;text-align:left;cursor:pointer;margin-bottom:2px}
  nav button:hover,nav button.active{background:var(--surface);border-color:var(--border)}
  nav strong{display:block;font-size:11px}nav span{display:block;font-size:9px;color:var(--faint);margin-top:3px}
  .empty-nav{font-size:10px;color:var(--faint);padding:6px}
  .detail{min-width:0;overflow:auto;padding:18px}
  form{max-width:520px;display:flex;flex-direction:column;gap:11px}
  .form-head{display:flex;align-items:center;justify-content:space-between}
  h2{font-size:13px;margin:0}
  label{display:block;font-size:10px;color:var(--muted)}
  input,textarea{display:block;width:100%;box-sizing:border-box;margin-top:5px;border:1px solid var(--border);border-radius:5px;background:var(--bg);color:var(--text);font:inherit;font-size:11px;padding:6px 7px;resize:vertical}
  input{height:29px;padding:0 7px}
  .pair{display:grid;grid-template-columns:1fr 1fr;gap:9px}
  .form-actions{display:flex;justify-content:flex-end;gap:8px;margin-top:4px}
  .danger,.view-actions button,.icon{height:29px;border:1px solid var(--border);border-radius:6px;background:var(--surface);color:var(--muted);padding:0 10px;display:flex;align-items:center;gap:6px;font:inherit;font-size:11px;cursor:pointer}
  .danger{color:#cc7a74}
  .icon{width:29px;padding:0;justify-content:center}
  .view{max-width:640px}
  .view-head{display:flex;align-items:flex-start;justify-content:space-between;gap:14px}
  .view-head p{font-size:11px;color:var(--muted);margin:5px 0 0;line-height:1.5}
  .view-actions{display:flex;gap:6px}
  .gallery{display:grid;grid-template-columns:repeat(auto-fill,minmax(104px,1fr));gap:9px;margin:16px 0}
  figure{margin:0;border:1px solid var(--border);border-radius:6px;overflow:hidden;background:var(--preview)}
  figure img{display:block;width:100%;height:96px;object-fit:contain;image-rendering:pixelated}
  figcaption{display:flex;align-items:center;justify-content:space-between;padding:4px 4px 4px 7px;border-top:1px solid var(--border);font-size:9px;color:var(--faint);background:var(--sidebar)}
  figcaption button{border:0;background:transparent;color:var(--faint);cursor:pointer;display:grid;place-items:center}
  figcaption button:hover{color:#cc7a74}
  dl{margin:14px 0 0;display:flex;flex-direction:column;gap:10px}
  dl div{display:grid;grid-template-columns:110px minmax(0,1fr);gap:10px}
  dt{font-size:9px;letter-spacing:.1em;color:var(--faint)}
  dd{margin:0;font-size:11px;color:var(--text);line-height:1.5}
  dd ul{margin:0;padding-left:15px}
  .swatches{display:flex;gap:4px;flex-wrap:wrap}
  .swatches i{width:16px;height:16px;border-radius:3px;border:1px solid var(--border)}
  .muted{font-size:11px;color:var(--faint);margin:16px 0 0;line-height:1.5}
  .empty{height:100%;display:flex;flex-direction:column;align-items:center;justify-content:center;color:var(--faint);text-align:center;gap:6px}
  .empty strong{font-size:12px;color:var(--text);margin-top:8px}
  .empty p{font-size:10px;margin:0 0 10px;max-width:340px;line-height:1.55}
</style>
