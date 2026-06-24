<script lang="ts">
  import type { ImagePanel } from "../../lib/types";
  import { assetUrl } from "../../lib/api";

  let { panel }: { panel: ImagePanel } = $props();
</script>

<div class="image">
  <img src={assetUrl(panel.asset_path)} alt={panel.alt ?? ""} />
</div>

<style>
  .image {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 100%;
  }
  img {
    max-width: 100%;
    max-height: 100%;
    object-fit: contain;
  }
</style>
