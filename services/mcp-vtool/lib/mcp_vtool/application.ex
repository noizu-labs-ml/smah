defmodule McpVtool.Application do
  @moduledoc """
  Supervision tree for the frozen-artifact replay service (ticket #23).

  The loopback TCP listener always runs; the stdio server joins the tree
  only when `MCPVTOOL_STDIO=1`. No OS processes are ever spawned — the
  service is pure file replay over JSON-RPC.
  """

  use Application

  @impl true
  def start(_type, _args) do
    children =
      [McpVtool.Server.Tcp] ++
        if System.get_env("MCPVTOOL_STDIO") == "1", do: [McpVtool.Server.Stdio], else: []

    Supervisor.start_link(children, strategy: :one_for_one, name: McpVtool.Supervisor)
  end
end
