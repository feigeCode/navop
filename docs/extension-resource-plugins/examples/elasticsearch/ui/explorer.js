import { View } from "gpui";
import { v_flex } from "gpui-base";
import { Button } from "gpui-component";
import { current } from "navop.context";
import { close, invoke, open } from "navop.resource";

export default class ElasticsearchExplorer extends View {
  init() {
    this.context = current();
    this.status = "Ready";
    this.resource = null;
    this.result = "";
  }

  render() {
    return v_flex()
      .size_full()
      .p(16)
      .gap(12)
      .child(`Extension: ${this.context.extensionId}`)
      .child(`Backends: ${this.context.backends.join(", ")}`)
      .child(`Status: ${this.status}`)
      .child(this.result)
      .child(
        new Button("load-cluster-info")
          .child("Load cluster info")
          .on_click((_event, cx) => {
            cx.spawn(async (cx) => {
              this.status = "Connecting";
              cx.notify();
              try {
                if (this.resource) await close(this.resource);
                const opened = await open("search", "elasticsearch", {
                  url: "http://elasticsearch.example.com:9200",
                });
                this.resource = opened.handle;
                const result = await invoke(
                  this.resource,
                  "elasticsearch/cluster/info",
                  {},
                );
                this.result = JSON.stringify(result, null, 2);
                this.status = "Connected";
              } catch (error) {
                this.status = `Failed: ${error.message}`;
              }
              cx.notify();
            });
          }),
      );
  }
}
