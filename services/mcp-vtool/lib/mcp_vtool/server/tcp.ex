defmodule McpVtool.Server.Tcp do
  @moduledoc """
  Loopback TCP transport: newline-delimited JSON-RPC 2.0, one connection
  at a time (sequential, like the Rust lane's `--tcp-port` mode).

  Binds `127.0.0.1` only — never a routable interface — on
  `MCPVTOOL_TCP_PORT` (default 4749); if the default is taken the server
  falls back to an ephemeral port and logs loudly. `bound_port/0` reports
  what actually bound.

  Lines are framed manually (accumulate to `\\n`) rather than via
  `packet: :line` + `line_length`: `line_length` cannot be applied to
  listen/accepted sockets portably, and the 4 MiB per-line cap must hold.
  A line over the cap answers one parse error and closes the connection.
  """

  use GenServer

  require Logger

  alias McpVtool.{Handler, JsonRpc}

  @line_length 4 * 1024 * 1024

  def start_link(args) do
    GenServer.start_link(__MODULE__, args, name: __MODULE__)
  end

  @doc "The port actually bound (resolves 0 to the ephemeral pick)."
  def bound_port do
    GenServer.call(__MODULE__, :port)
  end

  @impl true
  def init(args) do
    overrides = normalize(args)
    config = Map.merge(McpVtool.Tools.config(), overrides)

    requested =
      case Map.get(overrides, :port) do
        port when is_integer(port) -> port
        _ -> McpVtool.Tools.tcp_port()
      end

    listen(requested, config)
  end

  defp normalize([]), do: %{}
  defp normalize(%{} = map), do: map
  defp normalize(kw) when is_list(kw), do: Map.new(kw)

  defp listen(requested, config) do
    case :gen_tcp.listen(requested, listen_opts()) do
      {:ok, listen} ->
        {:ok, port} = :inet.port(listen)

        Logger.warning(
          "mcp-vtool: virtual MCP replay server on 127.0.0.1:#{port} (loopback only)"
        )

        spawn_link(fn -> acceptor(listen, config) end)
        {:ok, %{listen: listen, port: port, config: config}}

      {:error, :eaddrinuse} when requested != 0 ->
        Logger.warning(
          "mcp-vtool: port #{requested} busy — falling back to an ephemeral port " <>
            "(set MCPVTOOL_TCP_PORT to pin one)"
        )

        listen(0, config)

      {:error, reason} ->
        {:stop, reason}
    end
  end

  defp listen_opts do
    [
      :binary,
      active: false,
      reuseaddr: true,
      ip: {127, 0, 0, 1},
      backlog: 4
    ]
  end

  @impl true
  def handle_call(:port, _from, state), do: {:reply, state.port, state}

  # One connection served at a time; the accept loop blocks until the
  # current connection closes, then takes the next.
  defp acceptor(listen, config) do
    case :gen_tcp.accept(listen) do
      {:ok, socket} ->
        serve_connection(socket, config, "")
        acceptor(listen, config)

      {:error, :closed} ->
        :ok

      {:error, reason} ->
        Logger.error("mcp-vtool: accept failed: #{inspect(reason)}")
        :ok
    end
  end

  defp serve_connection(socket, config, buffer) do
    case :gen_tcp.recv(socket, 0) do
      {:ok, data} ->
        consume(buffer <> data, socket, config)

      {:error, _reason} ->
        :gen_tcp.close(socket)
    end
  end

  # Frame lines by hand: dispatch every complete line, park the partial
  # tail until more bytes arrive, and enforce the per-line cap.
  defp consume(data, socket, config) do
    case :binary.split(data, "\n") do
      [line, rest] ->
        case Handler.handle_line(<<line::binary, "\n">>, config) do
          "" -> :ok
          response -> :gen_tcp.send(socket, response)
        end

        consume(rest, socket, config)

      [partial] when byte_size(partial) > @line_length ->
        :gen_tcp.send(socket, JsonRpc.encode_error(nil, -32700, "line exceeds the 4 MiB cap"))
        :gen_tcp.close(socket)

      [partial] ->
        serve_connection(socket, config, partial)
    end
  end
end
