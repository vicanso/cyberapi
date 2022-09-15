import { defineComponent } from "vue";
import { css } from "@linaria/core";

const wrapperClass = css`
  padding: 15px;
  img,
  span,
  iframe {
    max-width: 100%;
    display: block;
    margin: auto;
  }
`;

export function isSupportPreview(contentType: string) {
  const reg = /image|pdf/i;
  return reg.test(contentType);
}

export default defineComponent({
  name: "ExPreview",
  props: {
    contentType: {
      type: String,
      required: true,
    },
    data: {
      type: String,
      required: true,
    },
  },
  render() {
    const { contentType, data } = this.$props;
    let dom = <p class="tac">Not Support</p>;
    const src = `data:${contentType};base64,${data}`;
    if (contentType.includes("image")) {
      dom = <img src={src} />;
    } else {
      const height = window.innerHeight || 700;
      dom = <iframe width={"100%"} height={`${height - 130}px`} src={src} />;
    }
    return <div class={wrapperClass}>{dom}</div>;
  },
});
