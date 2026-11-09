defmodule McpVtool.Fixtures do
  @moduledoc """
  Frozen-artifact fixtures shaped exactly like what the Rust lane freezes
  under `<cache>/smah/virtual-tools/<tool>/mcp/`.
  """

  @doc "Tempdir cache tree: `crm` with catalog + memo, `incomplete` without."
  def cache_tree do
    root = Path.join(System.tmp_dir!(), "mcp-vtool-test-#{:erlang.unique_integer([:positive])}")
    File.mkdir_p!(root)

    crm_dir = Path.join([root, "crm", "mcp"])
    File.mkdir_p!(crm_dir)

    File.write!(Path.join(crm_dir, "mcp-tools.json"), """
    [
      {
        "name": "search",
        "description": "search the contact list",
        "inputSchema": {
          "type": "object",
          "properties": {
            "q": {"type": "string"}
          },
          "required": ["q"]
        }
      },
      {
        "name": "fetch",
        "description": "fetch one contact by id",
        "inputSchema": {
          "type": "object",
          "properties": {
            "id": {"type": "integer"}
          }
        }
      }
    ]
    """)

    File.write!(Path.join(crm_dir, "mcp.sha256"), "deadbeef")

    File.write!(Path.join(crm_dir, "mcp-memo.jsonl"), """
    {"key":"#{McpVtool.Tools.memo_key("search", {:obj, [{"q", "ada"}]})}","name":"search","text":"found 2 contacts: ada, grace"}
    {"key":"#{McpVtool.Tools.memo_key("search", {:obj, [{"q", "grace"}]})}","name":"search","text":"found 1 contact: grace"}
    """)

    File.mkdir_p!(Path.join([root, "incomplete", "mcp"]))
    %{root: root, crm_dir: crm_dir}
  end

  def config(%{root: root, crm_dir: crm_dir}) do
    %{
      cache_dir: root,
      tools: ["crm"],
      tool: "crm",
      dir: crm_dir,
      catalog: McpVtool.Tools.load_catalog(crm_dir)
    }
  end

  def cleanup(%{root: root}), do: File.rm_rf!(root)
end
