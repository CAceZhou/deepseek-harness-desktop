<script lang="ts">
  import Splash from './splash/Splash.svelte'
  import Diagnostics from './diagnostics/Diagnostics.svelte'

  let route = $state(window.location.hash)
  window.addEventListener('hashchange', () => {
    route = window.location.hash
  })
</script>

{#if route.startsWith('#/diagnostics')}
  <Diagnostics />
{:else}
  <Splash />
{/if}
