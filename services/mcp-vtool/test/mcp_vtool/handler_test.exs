defmodule McpVtool.HandlerTest do
  use ExUnit.Case, async: false

  alias McpVtool.{Fixtures, Handler, Json, Tools}

  setup do
    tree = Fixtures.cache_tree()
    on_exit(fn -> Fixtures.cleanup(tree) end)
    {:ok, config: Fixtures.config(tree), tree: tree}
  end

  defp request(method, params, id), do: %{id: id, method: method, params: params}

  defp params(pairs), do: {:obj, pairs}

  test "initialize negotiates versions and names the tool", %{config: config} do
    {:reply, line} =
      Handler.handle(
        request("initialize", params([{"protocolVersion", "2025-06-18"}]), 1),
        config
      )

    assert {:ok, response} = Json.decode(line)

    assert {:ok, "2025-06-18"} =
             Json.get(response, "result") |> elem(1) |> Json.get("protocolVersion")

    {:reply, line} =
      Handler.handle(
        request("initialize", params([{"protocolVersion", "1999-01-01"}]), 2),
        config
      )

    assert {:ok, response} = Json.decode(line)
    {:ok, result} = Json.get(response, "result")
    assert {:ok, "2024-11-05"} = Json.get(result, "protocolVersion")
    {:ok, server_info} = Json.get(result, "serverInfo")
    assert {:ok, "smah-vtool-crm"} = Json.get(server_info, "name")
    {:ok, instructions} = Json.get(result, "instructions")
    assert instructions =~ "no fabrication"
  end

  test "ping returns an empty object", %{config: config} do
    assert {:reply, line} = Handler.handle(request("ping", params([]), 3), config)

    assert {:ok, {:obj, [{"jsonrpc", "2.0"}, {"id", 3}, {"result", {:obj, []}}]}} =
             Json.decode(line)
  end

  test "tools/list replays the frozen catalog", %{config: config} do
    {:reply, line} = Handler.handle(request("tools/list", params([]), 4), config)
    assert {:ok, response} = Json.decode(line)
    {:ok, result} = Json.get(response, "result")
    {:ok, tools} = Json.get(result, "tools")

    names =
      Enum.map(tools, fn tool ->
        {:ok, name} = Json.get(tool, "name")
        name
      end)

    assert names == ["search", "fetch"]
  end

  test "tools/call replays a memoized result", %{config: config} do
    params =
      params([{"name", "search"}, {"arguments", params([{"q", "ada"}])}])

    {:reply, line} = Handler.handle(request("tools/call", params, 5), config)
    assert {:ok, response} = Json.decode(line)
    {:ok, result} = Json.get(response, "result")
    {:ok, [content]} = Json.get(result, "content")
    assert {:ok, "found 2 contacts: ada, grace"} = Json.get(content, "text")
    assert {:ok, false} = Json.get(result, "isError")
  end

  test "tools/call answers isError with the fabricate hint on a memo miss", %{config: config} do
    params =
      params([{"name", "search"}, {"arguments", params([{"q", "nobody"}])}])

    {:reply, line} = Handler.handle(request("tools/call", params, 6), config)
    assert {:ok, response} = Json.decode(line)
    {:ok, result} = Json.get(response, "result")
    {:ok, [content]} = Json.get(result, "content")
    {:ok, text} = Json.get(content, "text")
    assert text =~ "not fabricated yet"
    assert text =~ "--no-serve"
    assert {:ok, true} = Json.get(result, "isError")
  end

  test "tools/call refuses unknown tools and missing names", %{config: config} do
    {:reply, line} =
      Handler.handle(request("tools/call", params([{"name", "nope"}]), 7), config)

    assert {:ok, {:obj, pairs}} = Json.decode(line)
    {:ok, error} = Json.get({:obj, pairs}, "error")
    assert {:ok, -32_602} = Json.get(error, "code")
    {:ok, message} = Json.get(error, "message")
    assert message =~ "no such tool 'nope' (available: search, fetch)"

    {:reply, line} = Handler.handle(request("tools/call", params([]), 8), config)
    assert {:ok, response} = Json.decode(line)
    {:ok, error} = Json.get(response, "error")
    assert {:ok, -32_602} = Json.get(error, "code")
  end

  test "unknown method answers METHOD_NOT_FOUND", %{config: config} do
    {:reply, line} = Handler.handle(request("resources/list", params([]), 9), config)
    assert {:ok, response} = Json.decode(line)
    {:ok, error} = Json.get(response, "error")
    assert {:ok, -32_601} = Json.get(error, "code")
  end

  test "notifications produce no response; notification-with-id is refused", %{config: config} do
    assert :no_response =
             Handler.handle(request("notifications/initialized", params([]), nil), config)

    {:reply, line} =
      Handler.handle(request("notifications/initialized", params([]), 10), config)

    assert {:ok, response} = Json.decode(line)
    {:ok, error} = Json.get(response, "error")
    assert {:ok, -32_600} = Json.get(error, "code")
  end

  test "handle_line end-to-end: request answered, notification silent, garbage an error", %{
    config: config
  } do
    assert "" =
             Handler.handle_line(
               ~s({"jsonrpc":"2.0","method":"notifications/initialized"}\n),
               config
             )

    line =
      Handler.handle_line(
        ~s({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"search","arguments":{"q":"ada"}}}\n),
        config
      )

    assert line =~ "found 2 contacts: ada, grace"
    assert String.ends_with?(line, "\n")

    error_line = Handler.handle_line("this is not json\n", config)
    assert {:ok, response} = Json.decode(error_line)
    assert {:ok, nil} = Json.get(response, "id")
    {:ok, error} = Json.get(response, "error")
    assert {:ok, -32_700} = Json.get(error, "code")
  end

  test "memo key parity: a call framed exactly as the Rust lane froze it replays", %{
    config: config
  } do
    # The fixture memo was frozen with sha256("search\0{\"q\":\"ada\"}")
    # (computed on the Rust side); the wire request must hit it.
    line =
      Handler.handle_line(
        ~s({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"search","arguments":{"q":"ada"}}}\n),
        config
      )

    assert line =~ ~s("isError":false)
    assert line =~ "found 2 contacts"
  end

  test "config resolution picks the discovered tool", %{tree: tree} do
    on_exit(fn ->
      System.delete_env("MCPVTOOL_CACHE_DIR")
      System.delete_env("MCPVTOOL_TOOL")
    end)

    System.put_env("MCPVTOOL_CACHE_DIR", tree.root)
    System.delete_env("MCPVTOOL_TOOL")
    config = Tools.config()
    assert config.tool == "crm"
    assert length(config.catalog) == 2

    System.put_env("MCPVTOOL_TOOL", "missing")
    config = Tools.config()
    assert config.tool == nil
    assert config.catalog == []
  end
end
