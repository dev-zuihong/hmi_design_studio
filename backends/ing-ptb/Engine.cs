using System.Security.Cryptography;
using System.Text;
using System.Text.Json.Nodes;
using IngPtbTool.Configuration;
using IngPtbTool.Models;
using IngPtbTool.Protocol;
using IngPtbTool.Services.Device;
using IngPtbTool.Services.Transport;

namespace IngPtb.Backend;

public sealed class Engine : IAsyncDisposable
{
    private sealed class Device(string key, PtbDeviceInfo info, JsonObject settings)
    {
        public string Key = key;
        public PtbDeviceInfo Info = info;
        public JsonObject Settings = settings;
        public JsonObject Draft = (JsonObject)settings.DeepClone();
        public PtbDeviceSession? Session;
        public byte Id;
        public string Version = "";
        public bool Present = true, Selected = true, Dirty, ResetRequired, HardwareRunning;
        public string State = "disconnected", Error = "";
        public string? JobId;
        public int Progress;
        public JsonObject? Result, ReadConfiguration;
        public JsonObject? Derived;
        public int Passed, Failed;
        public string LastActive = "-";
    }
    private sealed class Job(string id, string operation, Device[] devices)
    {
        public string Id = id, Operation = operation, State = "running", Error = "";
        public Device[] Devices = devices;
        public CancellationTokenSource Stop = new();
        public Task Completion = Task.CompletedTask;
        public string Started = DateTimeOffset.UtcNow.ToString("O");
        public string? Ended;
    }
    private readonly object gate = new();
    private readonly SemaphoreSlim scanGate = new(1, 1);
    private readonly ResourceRegistry resources = new();
    private readonly Dictionary<string, Device> devices = new(StringComparer.Ordinal);
    private readonly Dictionary<string, Job> jobs = new(StringComparer.Ordinal);
    private readonly Queue<JsonObject> logs = new();
    private readonly CancellationTokenSource lifetime = new();
    private readonly SettingsService settingsService;
    private readonly long generation = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() * 1000 + Random.Shared.Next(1000);
    private readonly string owner = Guid.NewGuid().ToString("N");
    private long sequence, logSequence;
    private string mode = "simulation";
    private JsonObject defaults = SettingsService.Defaults();
    private JsonObject deviceDefaults = new();
    private bool initialized, stopping;
    private bool autoExportLogs = true;
    private string logExportError = "";
    private readonly string logDirectory = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "HmiDesignStudio", "ing-ptb", "logs");
    private Task? discovery;
    public event Action<JsonObject>? Event;

    public Engine() { settingsService = new(resources); }

    public async Task<JsonObject> InitializeAsync(JsonObject parameters)
    {
        lock (gate) { if (initialized) throw new InvalidOperationException("Backend already initialized."); initialized = true; }
        mode = parameters["mode"]?.GetValue<string>() ?? "simulation";
        if (mode is not ("simulation" or "usb" or "real")) throw new InvalidDataException("mode必须为simulation或usb。");
        resources.Register(parameters["resources"] as JsonObject);
        var config = parameters["config"] as JsonObject ?? new();
        autoExportLogs = config["autoExportLogs"]?.GetValue<bool>() ?? true;
        if (config["defaultSettings"] is JsonObject supplied) defaults = CheckedSettings(supplied);
        deviceDefaults = config["deviceSettings"] as JsonObject ?? new();
        if (mode == "simulation")
        {
            int count = Math.Clamp(config["simulatedDeviceCount"]?.GetValue<int>() ?? 2, 1, 16);
            for (int index = 1; index <= count; index++)
            {
                string key = $"sim_{index}";
                devices.Add(key, NewDevice(key, new($"simulation:{index}", $"模拟 USB Printer {index}"), (byte)index));
            }
            await Task.WhenAll(devices.Values.Select(device => ConnectAsync(device, lifetime.Token)));
        }
        else await ScanAsync(lifetime.Token);
        Log("info", null, "后端已启动；兼容基线为当前 ING PTB GUI 三阶段协议。", null);
        discovery = Task.Run(DiscoveryLoopAsync);
        return Snapshot();
    }

    public JsonObject Snapshot()
    {
        lock (gate)
        {
            var rows = new JsonArray(); var byDevice = new JsonObject();
            foreach (var device in devices.Values)
            {
                var row = DeviceJson(device); rows.Add((JsonNode)row); byDevice[device.Key] = row.DeepClone();
            }
            var history = new JsonArray();
            foreach (var job in jobs.Values) history.Add((JsonNode)JobJson(job));
            JsonObject snapshot = new()
            {
                ["apiVersion"] = 1, ["generation"] = generation, ["sequence"] = sequence, ["mode"] = mode,
                ["autoExportLogs"] = autoExportLogs, ["logDirectory"] = logDirectory, ["logExportError"] = logExportError,
                ["devices"] = rows, ["byDevice"] = byDevice, ["jobs"] = history,
                ["logs"] = new JsonArray(logs.Select(item => (JsonNode)item.DeepClone()).ToArray()),
                ["logTotal"] = logs.Count, ["logTruncated"] = false,
                ["stats"] = new JsonObject { ["total"] = devices.Count, ["connected"] = devices.Values.Count(d => d.Session?.Transport.IsOpen == true && !d.Session.Faulted),
                    ["selected"] = devices.Values.Count(d => d.Selected), ["running"] = devices.Values.Count(d => d.JobId is not null),
                    ["idle"] = devices.Values.Count(d => d.State is "disconnected" or "ready" or "configured"),
                    ["burning"] = devices.Values.Count(d => d.State is "transferring" or "starting" or "running"), ["rfTesting"] = 0,
                    ["success"] = devices.Values.Count(d => d.State == "completed"), ["failure"] = devices.Values.Count(d => d.State is "failed" or "resetRequired"),
                    ["passed"] = devices.Values.Sum(d => d.Passed), ["failed"] = devices.Values.Sum(d => d.Failed) }
            };
            var visible = snapshot["logs"]!.AsArray();
            while (Encoding.UTF8.GetByteCount(snapshot.ToJsonString()) > 900 * 1024 && visible.Count > 0)
            {
                int remove = Math.Min(visible.Count, Math.Max(1, visible.Count / 10));
                for (int index = 0; index < remove; index++) visible.RemoveAt(0);
                snapshot["logTruncated"] = true;
            }
            return snapshot;
        }
    }

    private JsonObject DeviceJson(Device d)
    {
        d.Derived ??= settingsService.Describe(d.Draft);
        JsonObject value = new()
        {
        ["deviceKey"] = d.Key, ["deviceId"] = d.Id, ["name"] = $"PTB {d.Id}", ["location"] = d.Info.Location,
        ["targetChip"] = SettingsService.Parse(d.Settings).TargetChip.ToDisplayName(), ["lastActiveTime"] = d.LastActive,
        ["canActivate"] = !Duplicate(d) && !d.ResetRequired && d.JobId is null,
        ["showStartAction"] = !d.HardwareRunning && d.JobId is null, ["showStopAction"] = d.HardwareRunning || d.JobId is not null,
        ["stateText"] = Duplicate(d) ? "ID冲突" : d.State switch { "disconnected" => "未连接", "connecting" => "连接中", "ready" => "就绪", "transferring" => "下发中", "configured" => "已配置", "starting" => "启动中", "running" => "烧录中", "completed" => "成功", "cancelled" => "已取消", "resetRequired" => "需复位", "monitoringStopped" => "监控已停止", _ => "失败" },
        ["present"] = d.Present, ["selected"] = d.Selected, ["connected"] = d.Session?.Transport.IsOpen == true && !d.Session.Faulted,
        ["state"] = d.State, ["version"] = d.Version, ["error"] = d.Error, ["jobId"] = d.JobId, ["progress"] = d.Progress,
        ["resetRequired"] = d.ResetRequired || d.Session?.RequiresDeviceReset == true, ["hardwareRunning"] = d.HardwareRunning,
        ["duplicateId"] = Duplicate(d),
        ["settings"] = d.Settings.DeepClone(), ["draft"] = d.Draft.DeepClone(), ["dirty"] = d.Dirty,
        ["result"] = d.Result?.DeepClone(), ["readConfiguration"] = d.ReadConfiguration?.DeepClone(),
        ["passed"] = d.Passed, ["failed"] = d.Failed
        };
        foreach (var (key, data) in d.Derived) value[key] = data?.DeepClone();
        return value;
    }
    private static JsonObject JobJson(Job job) => new()
    {
        ["jobId"] = job.Id, ["operation"] = job.Operation, ["state"] = job.State, ["error"] = job.Error,
        ["deviceKeys"] = new JsonArray(job.Devices.Select(d => JsonValue.Create(d.Key)).ToArray()), ["startedAt"] = job.Started, ["endedAt"] = job.Ended
    };

    public Task<JsonNode?> InvokeAsync(JsonObject parameters)
    {
        lock (gate) { if (!initialized || stopping) throw new InvalidOperationException("Backend is not accepting operations."); }
        resources.Register(parameters["resources"] as JsonObject);
        string operation = parameters["operation"]?.GetValue<string>() ?? throw new InvalidDataException("operation必填。");
        var args = parameters["args"] as JsonObject ?? new();
        string? contextKey = parameters["context"]?["deviceKey"]?.GetValue<string>();
        JsonNode? result;
        switch (operation)
        {
            case "devices.scan":
                result = StartJob(operation, [], async (_, token) => await ScanAsync(token)); break;
            case "devices.select":
                lock (gate)
                {
                    if (args["deviceKeys"] is JsonArray selected)
                    {
                        var keys = selected.Select(k => k!.GetValue<string>()).ToHashSet(StringComparer.Ordinal);
                        if (keys.Any(k => !devices.ContainsKey(k))) throw new InvalidDataException("未知deviceKey。");
                        foreach (var device in devices.Values) device.Selected = keys.Contains(device.Key);
                    }
                    else RequireDevice(args, contextKey).Selected = args["selected"]?.GetValue<bool>() ?? true;
                }
                Changed(); result = Snapshot(); break;
            case "device.settings.get":
                lock (gate) result = SettingsJson(RequireDevice(args, contextKey)); break;
            case "device.settings.set":
            case "device.settings.patch":
            case "device.settings.revert":
            case "device.settings.apply":
                lock (gate)
                {
                    var d = RequireDevice(args, contextKey);
                    if (operation == "device.settings.revert") { d.Draft = (JsonObject)d.Settings.DeepClone(); d.Dirty = false; }
                    else if (operation == "device.settings.apply")
                    {
                        settingsService.Materialize(d.Draft, args["allowOverlap"]?.GetValue<bool>() ?? false);
                        d.Settings = CheckedSettings(d.Draft); d.Draft = (JsonObject)d.Settings.DeepClone(); d.Dirty = false;
                    }
                    else
                    {
                        var copy = (JsonObject)d.Draft.DeepClone();
                        if (operation == "device.settings.patch") Patch(copy, args["path"], args["value"]);
                        else Merge(copy, args["settings"] as JsonObject ?? throw new InvalidDataException("settings对象必填。"));
                        d.Draft = CheckedSettings(copy); d.Dirty = !JsonNode.DeepEquals(d.Draft, d.Settings);
                    }
                    d.Derived = null; result = SettingsJson(d);
                }
                Changed(); break;
            case "configuration.validate":
                lock (gate)
                {
                    var selected = args["settings"] ?? RequireDevice(args, contextKey).Draft;
                    try { bool overlap = args["allowOverlap"]?.GetValue<bool>() ?? false; var material = settingsService.Materialize(selected, overlap); result = new JsonObject { ["valid"] = true, ["packageBytes"] = DataPackageBuilder.Build(material, overlap).Length, ["errors"] = new JsonArray() }; }
                    catch (Exception error) when (error is InvalidDataException or ArgumentException or InvalidOperationException) { result = new JsonObject { ["valid"] = false, ["errors"] = new JsonArray(error.Message) }; }
                }
                break;
            case "configuration.import":
                lock (gate)
                {
                    JsonNode input = args["document"]?.DeepClone() ?? JsonNode.Parse(resources.Get(resources.ResolveId(args["resource"])).Read()) ?? throw new InvalidDataException("配置文件为空。");
                    var imported = settingsService.Import(input, args["references"] as JsonObject ?? args["resource"]?["references"] as JsonObject);
                    var device = RequireDevice(args, contextKey);
                    var draft = imported["settings"]!.AsObject();
                    if (imported["includesDutSettings"]!.GetValue<bool>() == false)
                        foreach (string field in new[] { "dutTargetChip", "dutFirmwareAddress", "dutFirmware" }) draft[field] = device.Draft[field]?.DeepClone();
                    device.Draft = CheckedSettings(draft); device.Dirty = !JsonNode.DeepEquals(device.Draft, device.Settings); device.Derived = null;
                    imported["settings"] = device.Draft.DeepClone(); result = imported;
                }
                Changed(); break;
            case "configuration.export":
                lock (gate)
                {
                    var d = RequireDevice(args, contextKey);
                    var exported = settingsService.Export(args["settings"] ?? (args["useDraft"]?.GetValue<bool>() == false ? d.Settings : d.Draft));
                    exported["name"] = args["name"]?.GetValue<string>() ?? $"ptb-{d.Id}.ptbcfg"; result = exported;
                }
                break;
            case "firmware.inspect": result = settingsService.Inspect(resources.ResolveId(args["resource"])); break;
            case "rolling.preview":
                lock (gate) result = SettingsService.RollingPreview(args["settings"] ?? RequireDevice(args, contextKey).Draft); break;
            case "logs.clear": lock (gate) logs.Clear(); Changed(); result = new JsonObject { ["cleared"] = true }; break;
            case "logs.setAutoExport":
                lock (gate) autoExportLogs = args["enabled"]?.GetValue<bool>() ?? throw new InvalidDataException("enabled必填。");
                Log("info", null, autoExportLogs ? "已启用每日自动日志导出。" : "已停用每日自动日志导出。", null);
                result = new JsonObject { ["autoExportLogs"] = autoExportLogs, ["logDirectory"] = logDirectory, ["logExportError"] = logExportError }; break;
            case "logs.export":
                lock (gate) result = new JsonObject { ["name"] = $"ptb-{DateTime.Now:yyyyMMdd-HHmmss}.jsonl", ["content"] = string.Join("\n", logs.Select(row => row.ToJsonString())) + "\n", ["resources"] = new JsonArray() }; break;
            case "jobs.cancel":
                lock (gate)
                {
                    string id = args["jobId"]?.GetValue<string>() ?? throw new InvalidDataException("jobId必填。");
                    if (!jobs.TryGetValue(id, out var job)) throw new InvalidDataException("未知jobId。"); job.Stop.Cancel(); result = JobJson(job);
                }
                break;
            case "task.stop":
                lock (gate)
                {
                    var targets = Targets(args, contextKey);
                    foreach (var job in jobs.Values.Where(j => j.State == "running" && j.Devices.Any(targets.Contains))) job.Stop.Cancel();
                    result = new JsonObject { ["accepted"] = true, ["message"] = "停止后续发送与结果监控；已发出的设备事务将收尾，PTB上的物理任务不保证停止。" };
                }
                break;
            case "device.connect": case "device.recover": case "device.disconnect": case "device.setId": case "device.readConfiguration":
            case "task.configure": case "task.sendDutFirmware": case "task.start": case "task.readResult":
                Device[] targetsForJob; Dictionary<string, JsonObject> captured;
                lock (gate)
                {
                    targetsForJob = Targets(args, contextKey);
                    captured = targetsForJob.ToDictionary(d => d.Key, d => (JsonObject)d.Settings.DeepClone());
                }
                if (operation == "device.recover" && args["resetConfirmed"]?.GetValue<bool>() != true) throw new InvalidDataException("恢复前必须确认设备已复位：resetConfirmed=true；重开USB句柄不等于复位。");
                result = StartJob(operation, targetsForJob, async (device, token) => await DeviceOperationAsync(operation, device!, captured[device!.Key], args, token));
                break;
            default: throw new InvalidDataException($"未知操作：{operation}");
        }
        return Task.FromResult<JsonNode?>(result);
    }

    private static JsonObject SettingsJson(Device device) => new() { ["settings"] = device.Settings.DeepClone(), ["draft"] = device.Draft.DeepClone(), ["dirty"] = device.Dirty };
    private Device RequireDevice(JsonObject args, string? contextKey)
    {
        string key = args["deviceKey"]?.GetValue<string>() ?? contextKey ?? throw new InvalidDataException("deviceKey必填，或由窗口context提供。");
        return devices.TryGetValue(key, out var device) ? device : throw new InvalidDataException($"未知设备：{key}");
    }
    private Device[] Targets(JsonObject args, string? contextKey)
    {
        Device[] target;
        if (args["deviceKeys"] is JsonArray keys) target = keys.Select(key => devices.TryGetValue(key!.GetValue<string>(), out var device) ? device : throw new InvalidDataException("未知deviceKey。")).Distinct().ToArray();
        else if (args["deviceKey"] is not null || contextKey is not null) target = [RequireDevice(args, contextKey)];
        else target = devices.Values.Where(d => d.Selected && d.Present).ToArray();
        if (target.Length == 0) throw new InvalidDataException("请先选择设备。");
        return target;
    }
    private JsonObject StartJob(string operation, Device[] targets, Func<Device?, CancellationToken, Task> work)
    {
        Job job;
        lock (gate)
        {
            if (stopping) throw new InvalidOperationException("Backend is shutting down.");
            if (targets.Any(d => d.JobId is not null)) throw new InvalidOperationException("DEVICE_BUSY: 设备已有任务，必须等待当前任务收尾。");
            while (jobs.Count >= 64)
            {
                var completed = jobs.Values.FirstOrDefault(j => j.State != "running") ?? throw new InvalidOperationException("任务数量超过上限。");
                jobs.Remove(completed.Id); completed.Stop.Dispose();
            }
            job = new(Guid.NewGuid().ToString("N"), operation, targets); jobs.Add(job.Id, job);
            foreach (var d in targets) { d.JobId = job.Id; d.Progress = 0; d.Error = ""; }
            job.Completion = Task.Run(async () =>
            {
                try
                {
                    if (targets.Length == 0) await work(null, job.Stop.Token);
                    else await Task.WhenAll(targets.Select(async device =>
                    {
                        try { await work(device, job.Stop.Token); }
                        catch (Exception error)
                        {
                            lock (gate)
                            {
                                if (error is not OperationCanceledException && operation == "task.start" && device.State is "starting" or "running") device.Failed++;
                                device.ResetRequired |= device.Session?.RequiresDeviceReset == true || error.Message.Contains("DEVICE_RESET_REQUIRED", StringComparison.Ordinal);
                                device.Error = error is OperationCanceledException ? "已停止后续发送；设备端任务可能仍在运行。" : Clip(error.Message, 512);
                                device.State = device.ResetRequired ? "resetRequired" : device.HardwareRunning ? "monitoringStopped" : error is OperationCanceledException ? "cancelled" : "failed";
                            }
                            Log(error is OperationCanceledException ? "warning" : "error", device.Key, device.Error, job.Id);
                            throw;
                        }
                    }));
                    lock (gate) job.State = "completed";
                }
                catch (OperationCanceledException) { lock (gate) { job.State = "cancelled"; job.Error = "已取消后续操作。"; } }
                catch (Exception error) { lock (gate) { job.State = "failed"; job.Error = Clip(error.Message, 512); } }
                finally
                {
                    lock (gate) { job.Ended = DateTimeOffset.UtcNow.ToString("O"); foreach (var d in targets) d.JobId = null; }
                    Changed(null, job.Id);
                }
            });
        }
        Changed(null, job.Id);
        return new() { ["accepted"] = true, ["jobId"] = job.Id };
    }

    private async Task DeviceOperationAsync(string operation, Device d, JsonObject captured, JsonObject args, CancellationToken token)
    {
        if (operation == "device.disconnect")
        {
            if (d.Session is not null) { d.ResetRequired |= d.Session.RequiresDeviceReset; await d.Session.DisposeAsync(); d.Session = null; }
            lock (gate) d.State = d.ResetRequired ? "resetRequired" : "disconnected";
            Changed(d.Key, d.JobId); return;
        }
        if (operation == "device.recover")
        {
            if (d.Session is not null) { await d.Session.DisposeAsync(); d.Session = null; }
            lock (gate) { d.ResetRequired = false; d.HardwareRunning = false; }
        }
        await ConnectAsync(d, token, operation == "device.recover");
        if (operation is "device.connect" or "device.recover") return;
        if (operation.StartsWith("task.", StringComparison.Ordinal) && operation != "task.readResult" && Duplicate(d)) throw new InvalidOperationException("设备ID重复，请先为设备分配唯一ID。");
        if (d.HardwareRunning && operation != "task.readResult") throw new InvalidOperationException("DEVICE_BUSY: 物理任务尚未确认完成，请读取结果或复位设备。");
        switch (operation)
        {
            case "device.setId":
                int id = args["deviceId"]?.GetValue<int>() ?? throw new InvalidDataException("deviceId必填。");
                if (id is < 1 or > 255) throw new InvalidDataException("deviceId必须为1至255。");
                lock (gate) if (devices.Values.Any(other => other != d && other.Present && other.Id == id)) throw new InvalidOperationException("该设备ID已被占用。");
                token.ThrowIfCancellationRequested(); MarkUncertain(d);
                var idReply = await d.Session!.RequestAsync(PtbCommand.DeviceId, [1, (byte)id], TimeSpan.FromSeconds(2), token); Ack(idReply); ClearUncertain(d);
                lock (gate) { d.Id = (byte)id; if (d.State is "failed" or "cancelled") d.State = "ready"; } break;
            case "device.readConfiguration":
                var response = await d.Session!.RequestAsync(PtbCommand.GetConfiguration, [], TimeSpan.FromSeconds(2), token);
                PtbDeviceSession.EnsureSuccess(response);
                if (response.Payload.Length != DataPackageBuilder.FixedHeaderSize + 1) throw new InvalidDataException("C6配置头长度无效。");
                var config = DataPackageBuilder.ParseSettings(response.Payload.AsSpan(1));
                var read = SettingsService.ToJson(new BurnConfigurationDocument { TargetChip = config.TargetChip, BurnMode = config.BurnMode,
                    EraseMode = config.EraseMode, ClearLock = config.ClearLock, LockEnabled = config.LockEnabled, TvccVoltage = config.TvccVoltage,
                    BurnCount = config.BurnCount, PostAction = config.PostAction, FirmwareFunction = config.FirmwareFunction, Rf = config.Rf, RollingCode = config.RollingCode });
                read["firmwareCount"] = response.Payload[74];
                read["firmwareContentsIncluded"] = false;
                lock (gate)
                {
                    d.ReadConfiguration = read;
                    if (d.State is "failed" or "cancelled") d.State = "ready";
                    if (args["applyToDraft"]?.GetValue<bool>() != false)
                    {
                        foreach (string field in new[] { "version", "targetChip", "burnMode", "eraseMode", "clearLock", "lockEnabled", "tvccVoltage", "burnCount", "postAction", "firmwareFunction", "rf", "rollingCode" })
                            d.Draft[field] = read[field]?.DeepClone();
                        d.Dirty = !JsonNode.DeepEquals(d.Draft, d.Settings); d.Derived = null;
                    }
                }
                Log("info", d.Key, "已读取C6配置头；保留本地固件引用与DUT页。", d.JobId); break;
            case "task.configure":
                var material = settingsService.Materialize(captured, args["allowOverlap"]?.GetValue<bool>() ?? false);
                await TransferAsync(d, PtbTransferFrameBuilder.CreateFrames(PtbCommand.Configure, DataPackageBuilder.Build(material, args["allowOverlap"]?.GetValue<bool>() ?? false)), token);
                lock (gate) d.State = "configured"; break;
            case "task.sendDutFirmware":
                var document = SettingsService.Parse(captured);
                var firmware = document.DutFirmware ?? throw new InvalidDataException("请先选择测试固件并保存设置。");
                var segment = settingsService.Load(firmware) with { DestinationAddress = document.DutFirmwareAddress };
                if (segment.Data.Length == 0 || (ulong)segment.DestinationAddress + (ulong)segment.Data.Length > uint.MaxValue + 1UL) throw new InvalidDataException("测试固件地址范围无效。");
                await TransferAsync(d, DutFirmwarePackageBuilder.CreateFrames(document.DutTargetChip, segment), token);
                lock (gate) d.State = "configured"; break;
            case "task.start":
                lock (gate) { d.State = "starting"; d.Result = null; }
                Changed(d.Key, d.JobId);
                token.ThrowIfCancellationRequested(); MarkUncertain(d);
                var start = await d.Session!.RequestAsync(PtbCommand.Start, [], TimeSpan.FromSeconds(2), token);
                if (start.Payload[0] == (byte)PtbResult.Busy) { lock (gate) d.HardwareRunning = true; }
                Ack(start); lock (gate) { d.HardwareRunning = true; d.State = "running"; }
                await ReadResultAsync(d, token); break;
            case "task.readResult": await ReadResultAsync(d, token); break;
        }
        Changed(d.Key, d.JobId);
    }

    private async Task ConnectAsync(Device device, CancellationToken token, bool resetConfirmed = false)
    {
        if (device.ResetRequired || device.Session?.RequiresDeviceReset == true) throw new InvalidOperationException("DEVICE_RESET_REQUIRED: 请先复位设备，然后确认恢复。");
        if (!device.Present) throw new IOException("设备已拔出。");
        if (device.Session?.Transport.IsOpen == true && !device.Session.Faulted) return;
        if (device.Session is not null) await device.Session.DisposeAsync();
        lock (gate) device.State = "connecting";
        IPtbTransport transport = mode == "simulation" ? new SimulatedTransport(device.Info, device.Id) : new UsbPrintTransport(device.Info, owner, resetConfirmed);
        var session = new PtbDeviceSession(transport); device.Session = session;
        session.Trace += (_, _) => { lock (gate) device.LastActive = DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss"); };
        session.Lost += message => { lock (gate) { device.Error = Clip(message, 512); device.ResetRequired |= session.RequiresDeviceReset; device.State = device.ResetRequired ? "resetRequired" : "disconnected"; } Log("error", device.Key, message, device.JobId); };
        session.Unsolicited += frame =>
        {
            if (frame.Command == PtbCommand.ReadResult && frame.Payload.Length == 21 && frame.Payload[0] == 0)
            {
                lock (gate) if (device.HardwareRunning) ApplyResult(device, PtbTestResult.Parse(frame.Payload.AsSpan(1)));
                Changed(device.Key, device.JobId);
            }
        };
        try
        {
            await session.ConnectAsync(token);
            string version = await session.GetVersionAsync(token); byte id = await session.GetDeviceIdAsync(token);
            lock (gate) { device.Version = version; device.Id = id; device.State = "ready"; device.Error = ""; }
            lock (gate) foreach (var duplicate in devices.Values.Where(Duplicate)) duplicate.Selected = false;
            Log("info", device.Key, $"已连接PTB，ID={id}，版本={version}。", device.JobId);
        }
        catch { device.ResetRequired |= session.RequiresDeviceReset; await session.DisposeAsync(); device.Session = null; throw; }
    }

    private async Task TransferAsync(Device device, IReadOnlyList<PtbFrame> frames, CancellationToken token)
    {
        bool started = false, completed = false;
        lock (gate) device.State = "transferring";
        try
        {
            for (int index = 0; index < frames.Count; index++)
            {
                token.ThrowIfCancellationRequested();
                if (index == 0) { MarkUncertain(device); started = true; }
                var request = frames[index];
                var response = await device.Session!.RequestAsync(request.Command, request.Payload, TimeSpan.FromSeconds(5), token);
                Ack(response);
                if (index == frames.Count - 1) { completed = true; ClearUncertain(device); }
                lock (gate) device.Progress = (index + 1) * 100 / frames.Count;
                if (index == 0 || completed || index % 64 == 0) Changed(device.Key, device.JobId);
            }
        }
        finally
        {
            if (started && !completed || device.Session?.RequiresDeviceReset == true)
            {
                lock (gate) device.ResetRequired = true;
                Log("warning", device.Key, "C3/C7传输未完成或结果不确定；禁止续传和自动重发，必须复位设备。", device.JobId);
            }
        }
    }

    private async Task ReadResultAsync(Device device, CancellationToken token)
    {
        DateTime deadline = DateTime.UtcNow.AddMinutes(5);
        bool awaitingRun;
        lock (gate) awaitingRun = device.HardwareRunning;
        while (DateTime.UtcNow < deadline)
        {
            token.ThrowIfCancellationRequested();
            lock (gate) if (awaitingRun && !device.HardwareRunning && device.Result is not null) return;
            var frame = await device.Session!.RequestAsync(PtbCommand.ReadResult, [], TimeSpan.FromSeconds(2), token);
            if (frame.Payload[0] == (byte)PtbResult.Busy) { await Task.Delay(500, token); continue; }
            PtbDeviceSession.EnsureSuccess(frame);
            if (frame.Payload.Length != 21) throw new InvalidDataException("C2结果必须恰好包含20字节test_result_t。");
            lock (gate) ApplyResult(device, PtbTestResult.Parse(frame.Payload.AsSpan(1)));
            Log("info", device.Key, device.Result!["passed"]!.GetValue<bool>() ? "设备任务完成：通过。" : "设备任务完成：未通过。", device.JobId);
            return;
        }
        throw new TimeoutException("设备结果监控达到300秒；PTB物理任务可能仍在运行，请继续读取结果或复位设备。");
    }
    private void ApplyResult(Device d, PtbTestResult result)
    {
        ClearUncertain(d);
        bool newResult = d.HardwareRunning || d.Result is null;
        bool passed = (!result.RfEnabled || (result.Rf.Status & 0x1F) == 0x1F) && (!result.FirmwareEnabled || result.Firmware.IsSuccess)
            && (!result.RollingCodeEnabled || result.RollingCode.IsSuccess) && (!result.LockEnabled || result.LockResult == 0);
        d.Result = new() { ["passed"] = passed, ["functionEnable"] = result.FunctionEnable,
            ["rf"] = new JsonObject { ["enabled"] = result.RfEnabled, ["txCount"] = result.Rf.TxCount, ["rxCount"] = result.Rf.RxCount, ["txPower"] = result.Rf.TxPower,
                ["rxSensitivity"] = result.Rf.RxSensitivity, ["frequencyOffset"] = result.Rf.FrequencyOffset, ["status"] = result.Rf.Status,
                ["txCountPassed"] = result.Rf.TxCountPassed, ["rxCountPassed"] = result.Rf.RxCountPassed, ["txPowerPassed"] = result.Rf.TxPowerPassed,
                ["rxSensitivityPassed"] = result.Rf.RxSensitivityPassed, ["frequencyOffsetPassed"] = result.Rf.FrequencyOffsetPassed },
            ["firmware"] = new JsonObject { ["enabled"] = result.FirmwareEnabled, ["result"] = result.Firmware.Result, ["address"] = result.Firmware.Address },
            ["rollingCode"] = new JsonObject { ["enabled"] = result.RollingCodeEnabled, ["result"] = result.RollingCode.Result, ["address"] = result.RollingCode.Address },
            ["lock"] = new JsonObject { ["enabled"] = result.LockEnabled, ["result"] = result.LockResult } };
        d.HardwareRunning = false; d.State = passed ? "completed" : "failed"; d.Progress = 100;
        if (newResult) { if (passed) d.Passed++; else d.Failed++; }
    }
    private static void MarkUncertain(Device device) { if (device.Session?.Transport is UsbPrintTransport usb) usb.Recovery.Mark(); }
    private static void ClearUncertain(Device device) { if (device.Session?.Transport is UsbPrintTransport usb) usb.Recovery.Clear(); }

    private async Task ScanAsync(CancellationToken token)
    {
        if (mode == "simulation") { Changed(); return; }
        await scanGate.WaitAsync(token);
        try
        {
            var found = new UsbPrintDeviceEnumerator().Enumerate().Take(16).ToArray();
            var paths = found.Select(d => d.DevicePath).ToHashSet(StringComparer.OrdinalIgnoreCase);
            List<Device> arrivals = [];
            bool changed = false;
            lock (gate)
            {
                foreach (var device in devices.Values)
                {
                    bool present = paths.Contains(device.Info.DevicePath);
                    changed |= device.Present != present;
                    if (device.Present && !present)
                    {
                        device.State = "disconnected"; device.Present = false;
                        if (device.JobId is not null && jobs.TryGetValue(device.JobId, out var job)) job.Stop.Cancel();
                    }
                    else { if (!device.Present && present && device.JobId is null && !device.ResetRequired) arrivals.Add(device); device.Present = present; }
                }
                foreach (var info in found)
                {
                    string key = "usb_" + Convert.ToHexStringLower(SHA256.HashData(Encoding.UTF8.GetBytes(info.DevicePath.ToUpperInvariant())))[..24];
                    if (!devices.TryGetValue(key, out var device))
                    {
                        if (devices.Count >= 16) continue;
                        device = NewDevice(key, info, 0); devices.Add(key, device); arrivals.Add(device);
                        changed = true;
                    }
                }
            }
            foreach (var device in arrivals)
            {
                token.ThrowIfCancellationRequested();
                try { StartJob("device.connect", [device], async (d, ct) => await ConnectAsync(d!, ct)); }
                catch (Exception error) { Log("warning", device.Key, error.Message, null); }
            }
            if (changed) Changed();
        }
        finally { scanGate.Release(); }
    }
    private async Task DiscoveryLoopAsync()
    {
        try
        {
            while (!lifetime.IsCancellationRequested)
            {
                await Task.Delay(650, lifetime.Token);
                if (mode != "simulation") try { await ScanAsync(lifetime.Token); } catch (Exception error) when (!lifetime.IsCancellationRequested) { Log("error", null, $"USB枚举失败：{error.Message}", null); }
            }
        }
        catch (OperationCanceledException) { }
    }
    private Device NewDevice(string key, PtbDeviceInfo info, byte id) => new(key, info,
        deviceDefaults[key] is JsonObject specific ? CheckedSettings(specific) : (JsonObject)defaults.DeepClone()) { Id = id };
    private bool Duplicate(Device device) => device.Id > 0 && devices.Values.Count(other => other.Present && other.Id == device.Id) > 1;
    private static void Ack(PtbFrame frame) { PtbDeviceSession.EnsureSuccess(frame); if (frame.Payload.Length != 1) throw new InvalidDataException("ACK长度必须为1。"); }
    private static JsonObject CheckedSettings(JsonObject node)
    {
        if (Encoding.UTF8.GetByteCount(node.ToJsonString()) > 8192) throw new InvalidDataException("设备设置超过8KiB限制。");
        return SettingsService.ToJson(SettingsService.Parse(node));
    }
    private static void Merge(JsonObject target, JsonObject patch)
    {
        foreach (var (key, value) in patch)
            if (value is JsonObject child && target[key] is JsonObject existing) Merge(existing, child); else target[key] = value?.DeepClone();
    }
    private void Patch(JsonObject target, JsonNode? pathNode, JsonNode? value)
    {
        string[] path = pathNode is JsonArray parts ? parts.Select(p => p!.GetValue<string>()).ToArray()
            : pathNode?.GetValue<string>().Split('.') ?? throw new InvalidDataException("path必填。");
        if (path.Length is < 1 or > 8 || path.Any(p => string.IsNullOrWhiteSpace(p) || p is "__proto__" or "prototype" or "constructor")) throw new InvalidDataException("设置路径无效。");
        string joined = string.Join('.', path);
        if (joined == "rollingCode.range" || joined == "dutFirmware.resource" || path[0] == "firmware" && path[^1] == "resource")
        { settingsService.PatchSpecial(target, joined, value); return; }
        if (joined == "dutFirmware.address") { path = ["dutFirmwareAddress"]; }
        else if (path[0] == "firmware" && path[^1] == "address") path[^1] = "destinationAddress";
        if (path[0] == "firmware" && path.Length == 3 && int.TryParse(path[1], out int slot) && slot is >= 0 and < 5)
        {
            var firmware = target["firmware"]!.AsArray();
            while (firmware.Count <= slot) firmware.Add((JsonNode)new JsonObject { ["enabled"] = false, ["destinationAddress"] = SettingsService.Parse(target).TargetChip.DefaultFirmwareAddress(), ["filePath"] = "" });
        }
        JsonNode current = target;
        for (int i = 0; i < path.Length - 1; i++) current = current is JsonArray array ? array[int.Parse(path[i])] ?? throw new InvalidDataException("设置路径不存在。") : current[path[i]] ?? throw new InvalidDataException("设置路径不存在。");
        JsonNode? prior = current is JsonArray a ? a[int.Parse(path[^1])] : current[path[^1]];
        if (value is JsonValue scalar && scalar.TryGetValue<string>(out string? text) && prior is JsonValue old)
        {
            if (old.GetValueKind() is System.Text.Json.JsonValueKind.True or System.Text.Json.JsonValueKind.False)
                value = JsonValue.Create(bool.Parse(text));
            else if (old.GetValueKind() == System.Text.Json.JsonValueKind.Number)
                value = text.StartsWith("0x", StringComparison.OrdinalIgnoreCase) ? JsonValue.Create(Convert.ToUInt64(text[2..], 16)) : JsonNode.Parse(text);
        }
        if (current is JsonArray list) list[int.Parse(path[^1])] = value?.DeepClone();
        else current[path[^1]] = value?.DeepClone();
        if (joined == "targetChip")
        {
            var chip = SettingsService.Parse(target).TargetChip;
            target["rollingCode"]!["address"] = chip.DefaultRollingCodeAddress();
            foreach (var item in target["firmware"]!.AsArray())
                if (item?["assetId"] is null && string.IsNullOrEmpty(item?["filePath"]?.GetValue<string>())) item!["destinationAddress"] = chip.DefaultFirmwareAddress();
        }
    }
    private void Log(string level, string? deviceKey, string message, string? jobId)
    {
        lock (gate)
        {
            message = Clip(message, 256);
            var entry = new JsonObject { ["id"] = ++logSequence, ["timestamp"] = DateTimeOffset.UtcNow.ToString("O"), ["level"] = level, ["deviceKey"] = deviceKey, ["jobId"] = jobId, ["message"] = message };
            logs.Enqueue(entry);
            while (logs.Count > 1000) logs.Dequeue();
            if (autoExportLogs)
            {
                try
                {
                    string directory = Path.Combine(logDirectory, DateTime.Now.ToString("yyyy-MM-dd")); Directory.CreateDirectory(directory);
                    File.AppendAllText(Path.Combine(directory, $"ptb-{owner}.jsonl"), entry.ToJsonString() + "\n", new UTF8Encoding(false)); logExportError = "";
                }
                catch (Exception error) when (error is IOException or UnauthorizedAccessException) { logExportError = error.Message; }
            }
        }
        Changed(deviceKey, jobId);
    }
    private void Changed(string? deviceKey = null, string? jobId = null)
    {
        lock (gate)
        {
            sequence++;
            Event?.Invoke(new JsonObject { ["kind"] = "snapshot", ["generation"] = generation, ["sequence"] = sequence, ["deviceKey"] = deviceKey, ["jobId"] = jobId, ["data"] = Snapshot() });
        }
    }
    private static string Clip(string value, int jsonBytes)
    {
        if (value.Length > jsonBytes) value = value[..jsonBytes];
        while (Encoding.UTF8.GetByteCount(JsonValue.Create(value)!.ToJsonString()) > jsonBytes) value = value[..^1];
        return value;
    }
    public async ValueTask DisposeAsync()
    {
        Job[] current;
        lock (gate) { if (stopping) return; stopping = true; lifetime.Cancel(); current = jobs.Values.ToArray(); foreach (var job in current) job.Stop.Cancel(); }
        if (discovery is not null) await discovery;
        await Task.WhenAll(current.Select(j => j.Completion));
        foreach (var device in devices.Values) if (device.Session is not null) await device.Session.DisposeAsync();
        foreach (var job in current) job.Stop.Dispose(); lifetime.Dispose();
    }
}
