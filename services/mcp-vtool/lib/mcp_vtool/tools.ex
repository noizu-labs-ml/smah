defmodule McpVtool.Tools do
  @moduledoc """
  Discovery of frozen virtual-MCP artifacts under the smah cache dir.

  The service never fabricates: it replays exactly what the Rust lane
  froze (`smah --emulate <tool> --no-serve` or any serving run):

      <cache>/smah/virtual-tools/<tool>/mcp/mcp-tools.json    (catalog)
      <cache>/smah/virtual-tools/<tool>/mcp/mcp-memo.jsonl    (call memo)

  `tools/list` replays the frozen catalog; `tools/call` replays the memo
  entry for `sha256(tool name <> 0x00 <> canonical arguments JSON)` — the
  same key the Rust lane froze — and answers `isError` with the fabricate-
  it hint when the artifact is missing.
  """

  @default_tcp_port 4749

  @doc "Runtime configuration resolved from the environment once at boot."
  def config do
    cache_dir = cache_dir()
    discovered = discover(cache_dir)

    tool =
      case System.get_env("MCPVTOOL_TOOL") do
        nil -> Enum.at(discovered, 0)
        name -> if name in discovered, do: name, else: nil
      end

    dir = tool_dir(cache_dir, tool)
    catalog = load_catalog(dir)
    %{cache_dir: cache_dir, tools: discovered, tool: tool, dir: dir, catalog: catalog}
  end

  @doc "Cache root: `MCPVTOOL_CACHE_DIR` or the XDG smah leaf."
  def cache_dir do
    case System.get_env("MCPVTOOL_CACHE_DIR") do
      dir when is_binary(dir) and dir != "" ->
        dir

      _ ->
        base =
          case System.get_env("XDG_CACHE_HOME") do
            home when is_binary(home) and home != "" -> home
            _ -> Path.expand("~/.cache")
          end

        Path.join(base, "smah/virtual-tools")
    end
  end

  @doc "Tools with a loadable frozen catalog, sorted."
  def discover(cache_dir) do
    case File.ls(cache_dir) do
      {:ok, entries} ->
        entries
        |> Enum.filter(fn entry ->
          cache_dir
          |> tool_dir(entry)
          |> catalog_path()
          |> File.regular?()
        end)
        |> Enum.sort()

      {:error, _} ->
        []
    end
  end

  def tool_dir(_cache_dir, nil), do: nil
  def tool_dir(cache_dir, tool), do: Path.join([cache_dir, tool, "mcp"])

  def catalog_path(nil), do: nil
  def catalog_path(dir), do: Path.join(dir, "mcp-tools.json")

  @doc "Load + validate a frozen catalog (a JSON array of tool objects)."
  def load_catalog(nil), do: []

  def load_catalog(dir) do
    with path when is_binary(path) <- catalog_path(dir),
         {:ok, raw} <- File.read(path),
         {:ok, entries} when is_list(entries) <- McpVtool.Json.decode(raw) do
      Enum.filter(entries, &valid_entry?/1)
    else
      _ -> []
    end
  end

  defp valid_entry?({:obj, pairs}) do
    Enum.any?(pairs, fn {key, value} -> key == "name" and is_binary(value) end)
  end

  defp valid_entry?(_other), do: false

  @doc "Memo path for a tool's mcp dir."
  def memo_path(nil), do: nil
  def memo_path(dir), do: Path.join(dir, "mcp-memo.jsonl")

  @doc """
  The memo key: `sha256(name <> 0x00 <> canonical arguments JSON)` —
  byte-identical to the Rust lane's `catalog::memo_key` (serde_json with
  preserve_order; canonical form = key order as written, minimal escapes).
  """
  def memo_key(name, arguments) do
    :crypto.hash(:sha256, name <> <<0>> <> McpVtool.Json.encode(arguments))
    |> Base.encode16(case: :lower)
  end

  @doc "Replay the newest memo entry for `key`, if any."
  def memo_get(dir, key) do
    with path when is_binary(path) <- memo_path(dir),
         {:ok, raw} <- File.read(path) do
      raw
      |> String.split("\n", trim: true)
      |> Enum.reverse()
      |> Enum.find_value(fn line ->
        case McpVtool.Json.decode(line) do
          {:ok, {:obj, pairs}} ->
            case List.keyfind(pairs, "key", 0) do
              {"key", ^key} ->
                case List.keyfind(pairs, "text", 0) do
                  {"text", text} when is_binary(text) -> {:ok, text}
                  _ -> nil
                end

              _ ->
                nil
            end

          _ ->
            nil
        end
      end)
      |> case do
        nil -> :error
        text -> text
      end
    else
      _ -> :error
    end
  end

  @doc "TCP port: `MCPVTOOL_TCP_PORT` or #{@default_tcp_port}."
  def tcp_port do
    case System.get_env("MCPVTOOL_TCP_PORT") do
      nil ->
        @default_tcp_port

      raw ->
        case Integer.parse(raw) do
          {port, ""} when port in 0..65_535 -> port
          _ -> @default_tcp_port
        end
    end
  end
end
