import '@fontsource-variable/manrope'
import '../../src/css/appearance.css'
if(location.search.includes('mobile'))await import('../../mobile/mobile.css')
import Preview from './Preview.svelte'
new Preview({target:document.getElementById('app')!})
