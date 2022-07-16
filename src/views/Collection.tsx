import { useMessage } from "naive-ui";
import { defineComponent, onBeforeMount, ref } from "vue";
import { useRoute } from "vue-router";
import { showError } from "../helpers/util";
import { useAPICollectionsStore } from "../stores/api_collection";
import ExLoading from "../components/ExLoading";
import { useHeaderStore } from "../stores/header";
import { APICollection } from "../commands/api_collection";

export default defineComponent({
  name: "CollectionView",
  setup() {
    const route = useRoute();
    const message = useMessage();
    const headerStore = useHeaderStore();
    const collection = ref({} as APICollection);

    const processing = ref(false);

    onBeforeMount(async () => {
      processing.value = true;
      try {
        const collectionStore = useAPICollectionsStore();
        const result = await collectionStore.get(route.query.id as string);
        if (result) {
          headerStore.add({
            name: result.name,
            route: route.name as string,
          });
          collection.value = result;
        }
      } catch (err) {
        showError(message, err);
      }
    });
    return {
      collection,
      processing,
    };
  },
  render() {
    const { processing } = this;
    if (processing) {
      return <ExLoading />;
    }
    return <p>abc</p>;
  },
});
