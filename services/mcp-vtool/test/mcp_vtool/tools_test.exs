defmodule McpVtool.ToolsTest do
  use ExUnit.Case, async: false

  alias McpVtool.{Fixtures, Tools}

  setup do
    tree = Fixtures.cache_tree()
    on_exit(fn -> Fixtures.cleanup(tree) end)
    {:ok, tree: tree}
  end

  test "memo keys match the Rust lane's frozen hashes byte-for-byte" do
    # Hardcoded sha256("search" <> 0x00 <> ~s({"q":"ada"})) — the value
    # serde_json (preserve_order) canonicalization produces on the Rust
    # side. The ordered codec must reproduce it exactly.
    assert Tools.memo_key("search", {:obj, [{"q", "ada"}]}) ==
             "aadc8217f7deaad4eb3c4be21e00a463b33b720d49f1cb968adc9a0f8f8cd041"

    # Multi-key arguments: order as written by the client, not sorted.
    assert Tools.memo_key("search", {:obj, [{"q", "grace"}, {"limit", 2}]}) ==
             "b9e4f187cdc1f2deff3a3db7fc9f62050fba4c0ce51fedd9d670693d2c0f04f9"
  end

  test "discover finds only tools with a frozen catalog", %{tree: tree} do
    assert Tools.discover(tree.root) == ["crm"]
  end

  test "load_catalog replays the frozen entries in file order", %{tree: tree} do
    catalog = Tools.load_catalog(tree.crm_dir)
    assert length(catalog) == 2

    names =
      Enum.map(catalog, fn entry ->
        {:ok, name} = McpVtool.Json.get(entry, "name")
        name
      end)

    assert names == ["search", "fetch"]

    schema = catalog |> hd() |> then(fn entry -> McpVtool.Json.get(entry, "inputSchema") end)
    assert match?({:ok, {:obj, _}}, schema)
  end

  test "load_catalog of a missing tool is empty", %{tree: tree} do
    assert Tools.load_catalog(Path.join([tree.root, "incomplete", "mcp"])) == []
    assert Tools.load_catalog(nil) == []
  end

  test "memo_get replays the newest entry for a key and misses cleanly", %{tree: tree} do
    key_ada = Tools.memo_key("search", {:obj, [{"q", "ada"}]})
    assert {:ok, "found 2 contacts: ada, grace"} = Tools.memo_get(tree.crm_dir, key_ada)

    key_missing = Tools.memo_key("search", {:obj, [{"q", "nobody"}]})
    assert :error = Tools.memo_get(tree.crm_dir, key_missing)
    assert :error = Tools.memo_get(nil, key_ada)
  end

  test "cache_dir honors MCPVTOOL_CACHE_DIR over XDG", %{tree: tree} do
    on_exit(fn ->
      System.delete_env("MCPVTOOL_CACHE_DIR")
      System.delete_env("XDG_CACHE_HOME")
    end)

    System.put_env("MCPVTOOL_CACHE_DIR", tree.root)
    assert Tools.cache_dir() == tree.root

    System.delete_env("MCPVTOOL_CACHE_DIR")
    System.put_env("XDG_CACHE_HOME", tree.root)
    assert Tools.cache_dir() == Path.join(tree.root, "smah/virtual-tools")
  end
end
