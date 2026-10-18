defmodule McpVtool.ServerTcpTest do
  use ExUnit.Case, async: false

  alias McpVtool.Fixtures

  @default_children {McpVtool.Server.Tcp, []}

  setup do
    tree = Fixtures.cache_tree()
    on_exit(fn -> Fixtures.cleanup(tree) end)

    # Swap the app-level listener onto the fixture tree, restoring the
    # default spec afterwards. The supervisor is one_for_one; the named
    # GenServer keeps its module name.
    swap_children(fixtures_spec(tree))

    on_exit(fn -> swap_children(@default_children) end)

    {:ok, port: McpVtool.Server.Tcp.bound_port(), tree: tree}
  end

  defp swap_children(spec) do
    sup = McpVtool.Supervisor
    Supervisor.terminate_child(sup, McpVtool.Server.Tcp)
    Supervisor.delete_child(sup, McpVtool.Server.Tcp)
    {:ok, _} = Supervisor.start_child(sup, spec)
    :ok
  end

  defp fixtures_spec(tree) do
    config = Fixtures.config(tree)

    %{
      id: McpVtool.Server.Tcp,
      start:
        {McpVtool.Server.Tcp, :start_link,
         [[tool: config.tool, dir: config.dir, catalog: config.catalog]]},
      restart: :permanent,
      type: :worker
    }
  end

  defp connect(port) do
    {:ok, socket} =
      :gen_tcp.connect(~c"127.0.0.1", port, [:binary, packet: :line, active: false])

    socket
  end

  defp send_recv(socket, line) do
    :ok = :gen_tcp.send(socket, line)
    {:ok, response} = :gen_tcp.recv(socket, 0)
    response
  end

  test "a full session replays the frozen artifacts over loopback TCP", %{port: port} do
    socket = connect(port)

    init =
      send_recv(
        socket,
        ~s({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}\n)
      )

    assert init =~ "smah-vtool-crm"

    # A notification: no response bytes for it.
    :ok = :gen_tcp.send(socket, ~s({"jsonrpc":"2.0","method":"notifications/initialized"}\n))

    assert send_recv(socket, ~s({"jsonrpc":"2.0","id":2,"method":"tools/list"}\n)) =~
             ~s("name":"search")

    call =
      send_recv(
        socket,
        ~s({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"search","arguments":{"q":"ada"}}}\n)
      )

    assert call =~ "found 2 contacts: ada, grace"
    assert call =~ ~s("isError":false)

    :ok = :gen_tcp.close(socket)

    # Sequential: the next connection is served after the first closes.
    socket2 = connect(port)
    assert send_recv(socket2, ~s({"jsonrpc":"2.0","id":9,"method":"ping"}\n)) =~ ~s("result":{})
    :ok = :gen_tcp.close(socket2)
  end

  test "protocol errors still answer on the wire", %{port: port} do
    socket = connect(port)

    assert send_recv(socket, "definitely not json\n") =~ ~s("code":-32700)

    assert send_recv(socket, ~s({"jsonrpc":"1.0","id":1,"method":"ping"}\n)) =~
             ~s("code":-32600)

    :ok = :gen_tcp.close(socket)
  end
end
