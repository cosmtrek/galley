<script lang="ts" module>
  import { PATHS, type IconName } from "../lib/icons";
  export type { IconName };
</script>

<script lang="ts">
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<svg class="glyph" viewBox="0 0 16 16" width={size} height={size} aria-hidden="true"><path d={PATHS[name]} /></svg>
