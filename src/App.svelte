<script lang="ts">
  import Splash from './splash/Splash.svelte'
  import Diagnostics from './diagnostics/Diagnostics.svelte'
  import Settings from './settings/Settings.svelte'

  let route = $state(window.location.hash)
  window.addEventListener('hashchange', () => {
    route = window.location.hash
  })
</script>

{#if route.startsWith('#/diagnostics')}
  <Diagnostics />
{:else if route.startsWith('#/settings')}
  <Settings />
{:else}
  <Splash />
{/if}
