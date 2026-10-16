using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json.Nodes;
using IngPtb.Backend;
using IngPtbTool.Configuration;
using IngPtbTool.Models;
using IngPtbTool.Protocol;
using IngPtbTool.Services.Device;
using IngPtbTool.Services.Transport;

int assertions = 0;
void Check(bool condition, string name) { if (!condition) throw new Exception("FAIL: " + name); assertions++; }
async Task Throws<T>(Func<Task> work, string name) where T : Exception
{ try { await work(); } catch (T) { assertions++; return; } throw new Exception("FAIL expected " + typeof(T).Name + ": " + name); }

byte[] golden = PtbFrameCodec.Encode(new(PtbFrame.HostType, PtbCommand.Start, []));
Check(Convert.ToHexString(golden) == "454A0401C1167E4E44", "C1 golden GUI wire CRC/order");
Check(Crc32Ieee.Compute(Encoding.ASCII.GetBytes("123456789")) == 0xCBF43926, "CRC32 golden");
var parser = new PtbFrameStreamParser();
Check(parser.Append([0, 1, 0x45]).Count == 0, "noise prefix and split header");
Check(parser.Append(golden.AsSpan(1, 4)).Count == 0, "incomplete frame retained");
Check(parser.Append(golden.AsSpan(5)).Single().Command == PtbCommand.Start, "split frame reconstructed");
var broken = (byte[])golden.Clone(); broken[5] ^= 0x20;
Check(parser.Append([.. broken, .. golden]).Count == 1, "bad CRC skipped before valid frame");

var config = new BurnConfiguration { FirmwareFunction = FirmwareFunction.Burn, Rf = new() { Enabled = true } };
config.Firmware.Add(new("image.bin", 0x02002000, Enumerable.Range(0, 131072).Select(i => (byte)i).ToArray()));
byte[] package = DataPackageBuilder.Build(config);
Check(package.Length == 94 + 8 + 131072, "C3 94-byte header and eight-byte record");
Check(DataPackageBuilder.Parse(package).Firmware[0].Data.SequenceEqual(config.Firmware[0].Data), "C3 package roundtrip");
var frames = PtbTransferFrameBuilder.CreateFrames(PtbCommand.Configure, package);
Check(frames.Count > 256 && frames.Skip(1).SkipLast(1).All(f => f.Payload.Length <= 254), "C3 supports more than256 frames,253 data bytes");
var c7 = DutFirmwarePackageBuilder.CreateFrames(PtbTargetChip.Ing920, config.Firmware[0]);
Check(c7[0].Payload.Length == 6 && c7[0].Payload[5] == 2, "C7 chip lives in Start");
Check(c7.Skip(1).SkipLast(1).Sum(f => f.Payload.Length - 1) == 131072 + 8, "C7 transfer excludes chip byte");

var transport = new SimulatedTransport(new("simulation:session", "test"), 7) { ResponseDelayMs = 0 };
await using (var session = new PtbDeviceSession(transport))
{
    await session.ConnectAsync(CancellationToken.None);
    Check(await session.GetVersionAsync(CancellationToken.None) == "1.3.1", "C5 version handshake");
    Check(await session.GetDeviceIdAsync(CancellationToken.None) == 7, "C4 get ID");
    foreach (var f in frames) PtbDeviceSession.EnsureSuccess(await session.RequestAsync(f.Command, f.Payload, TimeSpan.FromSeconds(1), CancellationToken.None));
    var header = await session.RequestAsync(PtbCommand.GetConfiguration, [], TimeSpan.FromSeconds(1), CancellationToken.None);
    Check(header.Payload.Length == 95 && DataPackageBuilder.ParseSettings(header.Payload.AsSpan(1)).FirmwareFunction == FirmwareFunction.Burn, "C6 header excludes firmware bodies");
    foreach (var f in c7) PtbDeviceSession.EnsureSuccess(await session.RequestAsync(f.Command, f.Payload, TimeSpan.FromSeconds(1), CancellationToken.None));
    Check(transport.WriteCount == frames.Count + c7.Count + 3, "each C3/C7 stage sent once");
    transport.ResponseDelayMs = 150;
    using var cancel = new CancellationTokenSource();
    int before = transport.WriteCount;
    var stopwatch = Stopwatch.StartNew();
    var pending = session.RequestAsync(PtbCommand.Start, [], TimeSpan.FromSeconds(1), cancel.Token);
    while (transport.WriteCount == before) await Task.Delay(1);
    cancel.Cancel();
    PtbDeviceSession.EnsureSuccess(await pending);
    Check(stopwatch.ElapsedMilliseconds >= 100 && !session.Faulted, "caller cancellation drains admitted request");
    await Throws<OperationCanceledException>(() => session.RequestAsync(PtbCommand.ReadResult, [], TimeSpan.FromSeconds(1), cancel.Token), "cancel prevents future writes");
    Check(transport.WriteCount == before + 1, "no additional write after cancellation");
}

var lost = new SimulatedTransport(new("simulation:timeout", "test"), 8) { DropNextReply = true, ResponseDelayMs = 0 };
await using (var session = new PtbDeviceSession(lost))
{
    await session.ConnectAsync(CancellationToken.None);
    await Throws<IOException>(() => session.RequestAsync(PtbCommand.Start, [], TimeSpan.FromMilliseconds(50), CancellationToken.None), "C1 ambiguous timeout");
    Check(lost.WriteCount == 1 && session.Faulted && session.RequiresDeviceReset, "ambiguous C1 timeout isolated without retry");
    await Throws<IOException>(() => session.RequestAsync(PtbCommand.Start, [], TimeSpan.FromMilliseconds(50), CancellationToken.None), "faulted session cannot consume late ACK");
    Check(lost.WriteCount == 1, "faulted connection never writes again");
}
foreach (var command in new[] { PtbCommand.Configure, PtbCommand.ConfigureDutFirmware })
{
    var uncertain = new SimulatedTransport(new("simulation:ambiguous", "test"), 9) { DropNextReply = true, ResponseDelayMs = 0 };
    await using var session = new PtbDeviceSession(uncertain);
    await session.ConnectAsync(CancellationToken.None);
    byte[] start = command == PtbCommand.Configure ? [0, 94, 0, 0, 0] : [0, 9, 0, 0, 0, 2];
    await Throws<IOException>(() => session.RequestAsync(command, start, TimeSpan.FromMilliseconds(50), CancellationToken.None), $"{command} ambiguous Start timeout");
    Check(uncertain.WriteCount == 1 && session.RequiresDeviceReset, $"{command} never blindly replays Start");
}
string fakeUsb = "test-recovery:" + Guid.NewGuid().ToString("N");
var firstOwner = new UsbRecoveryGuard(fakeUsb, "first-provider");
var restarted = new UsbRecoveryGuard(fakeUsb, "restarted-provider");
try
{
    firstOwner.Mark(); firstOwner.Check(false);
    await Throws<IOException>(() => { restarted.Check(false); return Task.CompletedTask; }, "persisted uncertain operation isolates a new process owner");
    restarted.Check(true); restarted.Check(false);
    Check(true, "explicit reset confirmation clears persisted recovery marker");
}
finally { firstOwner.Clear(); }

string directory = Path.Combine(Path.GetTempPath(), "hmi-ptb-tests-" + Guid.NewGuid().ToString("N"));
Directory.CreateDirectory(directory);
try
{
    string binary = Path.Combine(directory, "image.bin"); await File.WriteAllBytesAsync(binary, config.Firmware[0].Data);
    var registry = new ResourceRegistry();
    var refs = new JsonObject { ["fw_test"] = new JsonObject { ["path"] = binary, ["name"] = "image.bin", ["size"] = config.Firmware[0].Data.Length,
        ["sha256"] = Convert.ToHexStringLower(SHA256.HashData(config.Firmware[0].Data)) } };
    registry.Register(refs); var settings = new SettingsService(registry);
    Check(settings.Inspect("fw_test")["segments"]!.AsArray().Count == 1, "firmware asset reference inspected");
    var defaults = SettingsService.Defaults();
    foreach (int version in new[] { 2, 3, 4 })
    {
        var legacy = (JsonObject)defaults.DeepClone(); legacy["version"] = version; legacy["firmwareEnabled"] = false;
        var imported = settings.Import(legacy);
        Check(imported["settings"]!["version"]!.GetValue<int>() == 4 && imported["valid"]!.GetValue<bool>(), $"v{version} import migration");
        Check(imported["includesDutSettings"]!.GetValue<bool>() == (version >= 3), $"v{version} DUT settings presence");
    }
    settings.PatchSpecial(defaults, "firmware.0.resource", JsonValue.Create("fw_test")); defaults["firmwareFunction"] = 1;
    Check(settings.Materialize(defaults).Firmware[0].Data.Length == 131072, "resource-backed C3 materialization");
    var exported = settings.Export(defaults);
    Check(exported["content"]!.GetValue<string>().Contains("firmware/fw_test.bin"), "ptbcfg portable relative paths");
    Check(exported["resources"]!.AsArray().Count == 1, "export references companions instead of binary payload");
    var importedExport = settings.Import(JsonNode.Parse(exported["content"]!.GetValue<string>())!, new JsonObject { ["firmware/fw_test.bin"] = "fw_test" });
    Check(importedExport["valid"]!.GetValue<bool>() && settings.Materialize(importedExport["settings"]!).Firmware.Count == 1, "exported ptbcfg with companion mapping roundtrip");
    var rolling = SettingsService.RollingPreview(defaults)["rows"]!.AsArray();
    Check(rolling.Count == 52 && rolling[0]!["hex"]!.GetValue<string>() == "0000000000000000" && rolling[51]!["hex"]!.GetValue<string>() == "0000000000000033", "52 rolling previews using little-endian increment");
    settings.PatchSpecial(defaults, "rollingCode.range", JsonNode.Parse("{\"startByte\":2,\"byteCount\":3}"));
    Check(defaults["rollingCode"]!["bitmask"]!.GetValue<uint>() == 0x1C, "byte range to contiguous bitmask");
    await Throws<InvalidDataException>(() => { settings.PatchSpecial(defaults, "rollingCode.range", JsonNode.Parse("{\"startByte\":7,\"byteCount\":3}")); return Task.CompletedTask; }, "invalid byte range rejected");
    var described = settings.Describe(defaults);
    Check(described["firmwareRows"]!.AsArray().Count == 5 && described["memorySegments"]!.AsArray().Count == 2, "five slot firmware and RF memory map");

    await using var engine = new Engine();
    var initial = await engine.InitializeAsync(new JsonObject { ["mode"] = "simulation", ["resources"] = refs.DeepClone(), ["config"] = new JsonObject { ["autoExportLogs"] = false } });
    Check(initial["apiVersion"]!.GetValue<int>() == 1 && initial["devices"]!.AsArray().Count == 2 && initial["byDevice"]!["sim_1"] is not null, "snapshot root and keyed per-device bindings");
    Task<JsonNode?> Invoke(string op, JsonObject? args = null) => engine.InvokeAsync(new JsonObject { ["operation"] = op, ["args"] = args ?? new JsonObject { ["deviceKey"] = "sim_1" } });
    async Task<JsonNode> WaitJob(JsonNode accepted)
    {
        string jobId = accepted["jobId"]!.GetValue<string>(); var until = DateTime.UtcNow.AddSeconds(15);
        while (DateTime.UtcNow < until)
        {
            var job = engine.Snapshot()["jobs"]!.AsArray().Single(j => j!["jobId"]!.GetValue<string>() == jobId)!;
            if (job["state"]!.GetValue<string>() != "running") return job;
            await Task.Delay(10);
        }
        throw new Exception("job timed out in test");
    }
    Check((await WaitJob((await Invoke("device.connect"))!))["state"]!.GetValue<string>() == "completed", "connect async job");
    await Invoke("device.settings.patch", new() { ["deviceKey"] = "sim_1", ["path"] = "rf.frequencyMhz", ["value"] = "2404" });
    var device = engine.Snapshot()["byDevice"]!["sim_1"]!;
    Check(device["draft"]!["rf"]!["frequencyMhz"]!.GetValue<int>() == 2404 && device["settings"]!["rf"]!["frequencyMhz"]!.GetValue<int>() == 2402, "draft and committed settings isolated with string numeric coercion");
    Check(engine.Snapshot()["byDevice"]!["sim_2"]!["draft"]!["rf"]!["frequencyMhz"]!.GetValue<int>() == 2402, "second device settings isolated");
    await Invoke("device.settings.revert");
    Check(!engine.Snapshot()["byDevice"]!["sim_1"]!["dirty"]!.GetValue<bool>(), "revert discards draft");
    await Invoke("device.settings.set", new() { ["deviceKey"] = "sim_1", ["settings"] = defaults.DeepClone() }); await Invoke("device.settings.apply");
    var transfer = (await Invoke("task.configure"))!;
    await Throws<InvalidOperationException>(() => Invoke("task.start"), "whole transfer reserves device");
    var transferEnd = await WaitJob(transfer); Check(transferEnd["state"]!.GetValue<string>() == "completed", "full C3 async job");
    Check((await WaitJob((await Invoke("task.start"))!))["state"]!.GetValue<string>() == "completed", "C1 and busy C2 polls complete");
    Check(engine.Snapshot()["byDevice"]!["sim_1"]!["result"]!["passed"]!.GetValue<bool>(), "structured RF/burn result");
    Check((await WaitJob((await Invoke("task.readResult"))!))["state"]!.GetValue<string>() == "completed" && engine.Snapshot()["byDevice"]!["sim_1"]!["passed"]!.GetValue<int>() == 1, "repeated C2 observations do not double-count one physical task");
    Check((await WaitJob((await Invoke("device.readConfiguration"))!))["state"]!.GetValue<string>() == "completed", "C6 header read after firmware transfer");
    Check(engine.Snapshot()["byDevice"]!["sim_1"]!["draft"]!["firmware"]!.AsArray().Count == 1, "C6 refresh retains local firmware resources");
    var cancelled = (await Invoke("task.configure"))!;
    while (engine.Snapshot()["byDevice"]!["sim_1"]!["progress"]!.GetValue<int>() == 0) await Task.Delay(1);
    await Invoke("jobs.cancel", new() { ["jobId"] = cancelled["jobId"]!.DeepClone() });
    Check((await WaitJob(cancelled))["state"]!.GetValue<string>() == "cancelled", "C3 cancellation reports cancelled");
    Check(engine.Snapshot()["byDevice"]!["sim_1"]!["resetRequired"]!.GetValue<bool>(), "partial C3 requires physical reset");
    Check((await WaitJob((await Invoke("task.start"))!))["state"]!.GetValue<string>() == "failed", "partial C3 prevents later C1");
    await Throws<InvalidDataException>(() => Invoke("device.recover"), "recovery requires explicit reset confirmation");
    Check((await WaitJob((await Invoke("device.recover", new() { ["deviceKey"] = "sim_1", ["resetConfirmed"] = true }))!))["state"]!.GetValue<string>() == "completed", "confirmed simulator reset creates fresh transport");
    for (int index = 0; index < 1005; index++) await Invoke("logs.setAutoExport", new() { ["enabled"] = false });
    var logSnapshot = engine.Snapshot();
    Check(logSnapshot["logTotal"]!.GetValue<int>() == 1000 && logSnapshot["logs"]!.AsArray().Count == 1000, "business log retains and displays1000 entries");
    var logExport = (await Invoke("logs.export"))!;
    Check(logExport["content"]!.GetValue<string>().Split('\n', StringSplitOptions.RemoveEmptyEntries).Length == 1000, "manual log export includes entire1000-entry ring");
    Check(Encoding.UTF8.GetByteCount(logExport.ToJsonString()) < IngPtb.Backend.Program.MaxLineBytes, "1000-entry log export fits RPC budget");
    var daily = (await Invoke("logs.setAutoExport", new() { ["enabled"] = true }))!;
    Check(daily["logExportError"]!.GetValue<string>() == "" && Directory.EnumerateFiles(Path.Combine(daily["logDirectory"]!.GetValue<string>(), DateTime.Now.ToString("yyyy-MM-dd")), "*.jsonl").Any(), "daily log toggle writes user-data JSONL");
    await Invoke("logs.setAutoExport", new() { ["enabled"] = false });
    var missing = settings.Import(JsonNode.Parse("{\"version\":4,\"firmware\":[{\"filePath\":\"missing.bin\",\"destinationAddress\":33562624}]}")!);
    Check(!missing["valid"]!.GetValue<bool>() && missing["missingResources"]!.AsArray().Count == 1, "missing companion reported explicitly");
    await File.AppendAllTextAsync(binary, "changed");
    await Throws<InvalidDataException>(() => { registry.Get("fw_test").Read(); return Task.CompletedTask; }, "asset content cannot silently change");
}
finally { Directory.Delete(directory, true); }
Console.WriteLine($"PASS: {assertions} protocol, resource, configuration, lifecycle and isolation assertions.");
