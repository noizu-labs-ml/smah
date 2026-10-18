defmodule McpVtool.Handler do
  @moduledoc """
  Method dispatch for the frozen-artifact replay server (ticket #23).

  Methods: `initialize`, `ping`, `tools/list`, `tools/call`, plus
  notification tolerance. No fabrication happens here — `tools/list`
  replays the frozen catalog, `tools/call` replays the memo, and a miss
  answers `isError` with the fabricate-it hint. Transport-agnostic: the
  stdio and TCP servers both route lines through `handle_line/2`.
  """

  @supported_protocol_versions ["2024-11-05", "2025-06-18"]
  @fabricate_hint "not fabricated yet — run `smah --emulate <tool> --no-serve`"

  alias McpVtool.{Json, JsonRpc, Tools}

  @doc """
  One transport line in, zero or one transport lines out (a notification
  produces no response). Returns the encoded response line (or "").
  """
  def handle_line(line, config) do
    case JsonRpc.parse_line(line) do
      {:ok, request} ->
        case handle(request, config) do
          :no_response -> ""
          {:reply, reply} -> reply
        end

      {:error, id, code, message} ->
        JsonRpc.encode_error(id, code, message)
    end
  end

  @doc "Dispatch one parsed request. Notifications yield `:no_response`."
  def handle(%{id: nil}, _config), do: :no_response

  def handle(%{id: id, method: method} = request, config) do
    if String.starts_with?(method, "notifications/") do
      {:reply,
       JsonRpc.encode_error(id, JsonRpc.invalid_request(), "notifications must not carry an id")}
    else
      dispatch(id, method, request.params, config)
    end
  end

  defp dispatch(id, "initialize", params, config) do
    requested =
      case Json.get(params, "protocolVersion") do
        {:ok, version} when is_binary(version) -> version
        _ -> ""
      end

    version =
      if requested in @supported_protocol_versions,
        do: requested,
        else: Enum.at(@supported_protocol_versions, 0)

    server_name = "smah-vtool-#{config.tool || "none"}"

    {:reply,
     JsonRpc.encode_result(
       id,
       {:obj,
        [
          {"protocolVersion", version},
          {"capabilities", {:obj, [{"tools", {:obj, [{"listChanged", false}]}}]}},
          {"serverInfo", {:obj, [{"name", server_name}, {"version", vsn()}]}},
          {"instructions",
           "Replay of the frozen virtual MCP catalog for '#{config.tool || "none"}'; " <>
             "no fabrication happens in this process."}
        ]}
     )}
  end

  defp dispatch(id, "ping", _params, _config) do
    {:reply, JsonRpc.encode_result(id, {:obj, []})}
  end

  defp dispatch(id, "tools/list", _params, config) do
    {:reply, JsonRpc.encode_result(id, {:obj, [{"tools", config.catalog}]})}
  end

  defp dispatch(id, "tools/call", params, config) do
    name =
      case Json.get(params, "name") do
        {:ok, name} when is_binary(name) -> name
        _ -> nil
      end

    arguments =
      case Json.get(params, "arguments") do
        {:ok, {:obj, _} = arguments} -> arguments
        _ -> {:obj, []}
      end

    cond do
      is_nil(name) ->
        error(id, JsonRpc.invalid_params(), "'name' must be a string")

      entry(config, name) == nil ->
        available = config.catalog |> tool_names() |> Enum.join(", ")
        error(id, JsonRpc.invalid_params(), "no such tool '#{name}' (available: #{available})")

      true ->
        replay_call(id, config, name, arguments)
    end
  end

  defp dispatch(id, method, _params, _config) do
    error(
      id,
      JsonRpc.method_not_found(),
      "unknown method '#{method}' (supported: initialize, ping, tools/list, tools/call)"
    )
  end

  defp replay_call(id, config, name, arguments) do
    key = Tools.memo_key(name, arguments)

    case Tools.memo_get(config.dir, key) do
      {:ok, text} ->
        {:reply, JsonRpc.encode_result(id, call_result(text, false))}

      :error ->
        {:reply, JsonRpc.encode_result(id, call_result(@fabricate_hint, true))}
    end
  end

  defp call_result(text, is_error) do
    {:obj,
     [
       {"content", [{:obj, [{"type", "text"}, {"text", text}]}]},
       {"isError", is_error}
     ]}
  end

  defp entry(config, name) do
    Enum.find(config.catalog, fn entry ->
      match?({:ok, ^name}, Json.get(entry, "name"))
    end)
  end

  defp tool_names(catalog) do
    Enum.map(catalog, fn entry ->
      case Json.get(entry, "name") do
        {:ok, name} when is_binary(name) -> name
        _ -> "?"
      end
    end)
  end

  defp error(id, code, message), do: {:reply, JsonRpc.encode_error(id, code, message)}

  defp vsn do
    case Application.spec(:mcp_vtool, :vsn) do
      nil -> "0.0.0"
      vsn -> to_string(vsn)
    end
  end
end
