import { defineComponent } from "vue";

import "./main.css";
import APISettings from "./components/APISettings";

export default defineComponent({
  name: "App",
  render() {
    return <APISettings />;
  },
});
