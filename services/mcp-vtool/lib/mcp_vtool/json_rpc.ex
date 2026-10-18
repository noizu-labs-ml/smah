defmodule McpVtool.JsonRpc do
  @moduledoc """
  JSON-RPC 2.0 line framing for the virtual MCP server (ticket #23).

  Requests arrive newline-delimited; responses leave the same way.
  Requests without an `id` are notifications and produce no response —
  mirroring the Rust lane, a `notifications/*` method carrying an id is
  refused with INVALID_REQUEST instead.
  """

  @parse_error -32_700
  @invalid_request -32_600
  @method_not_found -32_601
  @invalid_params -32_602
  @internal_error -32_603

  def parse_error, do: @parse_error
  def invalid_request, do: @invalid_request
  def method_not_found, do: @method_not_found
  def invalid_params, do: @invalid_params
  def internal_error, do: @internal_error

  @doc """
  Parse one transport line. Returns `{:ok, request}` with

      %{id: id | nil, method: binary, params: json}

  or `{:error, id | nil, code, message}` for protocol-level failures.
  """
  @spec parse_line(String.t()) ::
          {:ok, %{id: term() | nil, method: String.t(), params: term()}}
          | {:error, term() | nil, integer(), String.t()}
  def parse_line(line) do
    case McpVtool.Json.decode(line) do
      {:error, reason} ->
        {:error, nil, @parse_error, "parse error: #{reason}"}

      {:ok, decoded} ->
        validate(decoded)
    end
  end

  defp validate({:obj, _pairs} = decoded) do
    id = id_of(decoded)

    jsonrpc =
      case McpVtool.Json.get(decoded, "jsonrpc") do
        {:ok, "2.0"} -> :ok
        _ -> :missing
      end

    method =
      case McpVtool.Json.get(decoded, "method") do
        {:ok, method} when is_binary(method) -> {:ok, method}
        _ -> :missing
      end

    case {jsonrpc, method} do
      {:ok, {:ok, method}} ->
        params =
          case McpVtool.Json.get(decoded, "params") do
            {:ok, params} -> params
            :error -> {:obj, []}
          end

        {:ok, %{id: id, method: method, params: params}}

      _ ->
        {:error, id, @invalid_request, "request must carry jsonrpc \"2.0\" and a string method"}
    end
  end

  defp validate(_other) do
    {:error, nil, @invalid_request, "request must be a JSON object"}
  end

  defp id_of(decoded) do
    case McpVtool.Json.get(decoded, "id") do
      {:ok, id} -> id
      :error -> nil
    end
  end

  @doc "Encode a success response line (with trailing newline)."
  @spec encode_result(term(), term()) :: String.t()
  def encode_result(id, result) do
    McpVtool.Json.encode({:obj, [{"jsonrpc", "2.0"}, {"id", id}, {"result", result}]}) <> "\n"
  end

  @doc "Encode an error response line (with trailing newline)."
  @spec encode_error(term() | nil, integer(), String.t()) :: String.t()
  def encode_error(id, code, message) do
    McpVtool.Json.encode(
      {:obj,
       [
         {"jsonrpc", "2.0"},
         {"id", id},
         {"error", {:obj, [{"code", code}, {"message", message}]}}
       ]}
    ) <> "\n"
  end
end
