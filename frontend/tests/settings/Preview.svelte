<script>
    import {SvelteUIProvider} from '@svelteuidev/core'
    import Settings from '../../src/routes/settings/index.svelte'
    import {resolvedAppearance} from '../../src/lib/appearance'
</script>
<SvelteUIProvider themeObserver={$resolvedAppearance} withNormalizeCSS withGlobalStyles><Settings/></SvelteUIProvider>
