using System.Text;
using System.Text.Json.Nodes;
using System.Threading.Channels;

namespace IngPtb.Backend;

public static class Program
{
    public const int MaxLineBytes = 1024 * 1024;
    public static async Task<int> Main()
    {
        Console.InputEncoding = new UTF8Encoding(false, true); Console.OutputEncoding = new UTF8Encoding(false);
        var outgoing = Channel.CreateBounded<JsonObject>(new BoundedChannelOptions(128) { SingleReader = true, FullMode = BoundedChannelFullMode.Wait });
        using var stop = new CancellationTokenSource();
        var writer = Task.Run(async () =>
        {
            await foreach (var value in outgoing.Reader.ReadAllAsync())
            {
                string line = value.ToJsonString();
                if (Encoding.UTF8.GetByteCount(line) > MaxLineBytes) throw new InvalidDataException("Outgoing JSON-RPC line exceeded 1MiB.");
                await Console.Out.WriteLineAsync(line); await Console.Out.FlushAsync();
            }
        });
        await using var engine = new Engine();
        // Snapshots are complete and monotonic. Under stdout backpressure redundant
        // snapshots may be dropped; RPC responses remain lossless, snapshot resync is supported.
        engine.Event += data => outgoing.Writer.TryWrite(new JsonObject { ["jsonrpc"] = "2.0", ["method"] = "event", ["params"] = data });
        try
        {
            await foreach (string line in ReadLinesAsync(Console.OpenStandardInput(), stop.Token))
            {
                JsonNode? id = null; bool shutdown = false;
                JsonObject response;
                try
                {
                    var request = JsonNode.Parse(line)?.AsObject() ?? throw new InvalidDataException("RPC request must be an object.");
                    id = request["id"]?.DeepClone();
                    if (request["jsonrpc"]?.GetValue<string>() != "2.0" || id is null) throw new InvalidDataException("RPC requires jsonrpc=2.0 and id.");
                    var parameters = request["params"] as JsonObject ?? new();
                    string method = request["method"]?.GetValue<string>() ?? "";
                    JsonNode? result;
                    switch (method)
                    {
                        case "initialize": result = await engine.InitializeAsync(parameters); break;
                        case "snapshot": result = engine.Snapshot(); break;
                        case "invoke": result = await engine.InvokeAsync(parameters); break;
                        case "shutdown": await engine.DisposeAsync(); result = new JsonObject { ["stopped"] = true }; shutdown = true; break;
                        default: throw new InvalidDataException($"Unknown RPC method: {method}");
                    }
                    response = new() { ["jsonrpc"] = "2.0", ["id"] = id, ["result"] = result };
                }
                catch (Exception error)
                {
                    response = new() { ["jsonrpc"] = "2.0", ["id"] = id, ["error"] = new JsonObject { ["code"] = -32000, ["message"] = error.Message.Length > 2048 ? error.Message[..2048] : error.Message } };
                }
                await outgoing.Writer.WriteAsync(response, stop.Token);
                if (shutdown) break;
                if (writer.IsFaulted) await writer;
            }
            await engine.DisposeAsync(); outgoing.Writer.Complete(); await writer; return 0;
        }
        catch (Exception error)
        {
            await Console.Error.WriteLineAsync(error.Message); outgoing.Writer.TryComplete(); return 1;
        }
    }

    private static async IAsyncEnumerable<string> ReadLinesAsync(Stream input, [System.Runtime.CompilerServices.EnumeratorCancellation] CancellationToken token)
    {
        byte[] chunk = new byte[8192]; using var line = new MemoryStream();
        var utf8 = new UTF8Encoding(false, true);
        while (true)
        {
            int count = await input.ReadAsync(chunk, token);
            if (count == 0) { if (line.Length > 0) throw new InvalidDataException("Incomplete JSON-RPC line at EOF."); yield break; }
            for (int index = 0; index < count; index++)
            {
                byte value = chunk[index];
                if (value == '\n') { string text = utf8.GetString(line.GetBuffer(), 0, (int)line.Length).TrimEnd('\r'); line.SetLength(0); if (text.Length > 0) yield return text; }
                else { if (line.Length >= MaxLineBytes) throw new InvalidDataException("JSON-RPC line exceeds 1MiB."); line.WriteByte(value); }
            }
        }
    }
}
