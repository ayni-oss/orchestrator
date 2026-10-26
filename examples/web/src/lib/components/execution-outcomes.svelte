<script lang="ts">
import InformationTable from "$lib/components/information-table.svelte";
import {words} from "$lib/presentation.js";
let {execution,quality,publication,progress}:{execution:string;quality:string;publication:string;progress?:string}=$props();
</script>
<InformationTable rows={[...(progress?[{label:"Progress",value:words(progress)}]:[]),{label:"Execution",value:words(execution)},{label:"Quality",value:words(quality)},{label:"Publication",value:words(publication)}]} />
