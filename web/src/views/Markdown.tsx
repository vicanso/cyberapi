import { defineComponent, onBeforeMount, ref } from "vue";
import { css } from "@linaria/core";
import { useRoute } from "vue-router";
import { useMessage } from "naive-ui";
import { showError } from "../helpers/util";
import request from "../helpers/request";

import { MAIN_CONTENT_WIDTH } from "../constant";

const markdownClass = css`
  margin: auto;
  width: ${MAIN_CONTENT_WIDTH}px;
  img {
    max-width: 100%;
  }
`;

export default defineComponent({
  name: "MarkdownView",
  setup() {
    const message = useMessage();
    const route = useRoute();
    const html = ref("");

    onBeforeMount(async () => {
      try {
        const { data } = await request.get<{
          html: string;
        }>(`/markdowns/v1/${route.params.name}`);
        html.value = data.html;
      } catch (err) {
        showError(message, err);
      }
    });
    return {
      html,
    };
  },
  render() {
    const { html } = this;
    return <div class={markdownClass} v-html={html}></div>;
  },
});
