import { defineComponent } from "vue";
import { css } from "@linaria/core";

const footerClass = css`
  text-align: center;
  margin: 15px 0;
`;

export default defineComponent({
  name: "AppFooter",
  render() {
    return (
      <footer class={footerClass}>© 2022 Tree Xie. All rights reserved.</footer>
    );
  },
});
