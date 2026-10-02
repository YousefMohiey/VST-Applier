namespace VstApplier
{
    partial class MainForm
    {
        /// <summary>
        ///  Required designer variable.
        /// </summary>
        private System.ComponentModel.IContainer components = null;

        /// <summary>
        ///  Clean up any resources being used.
        /// </summary>
        /// <param name="disposing">true if managed resources should be disposed; otherwise, false.</param>
        protected override void Dispose(bool disposing)
        {
            if (disposing && (components != null))
            {
                components.Dispose();
            }

            base.Dispose(disposing);
        }

        #region Windows Form Designer generated code

        /// <summary>
        ///  Required method for Designer support - do not modify
        ///  the contents of this method with the code editor.
        /// </summary>
        private void InitializeComponent()
        {
            leftPanel = new Panel();
            outputLevelGroupBox = new GroupBox();
            outputLevelStatusLabel = new Label();
            outputLevelMeter = new AudioLevelMeterControl();
            inputLevelGroupBox = new GroupBox();
            inputLevelStatusLabel = new Label();
            inputLevelMeter = new AudioLevelMeterControl();
            virtualCableGroupBox = new GroupBox();
            latencyStatusValueLabel = new Label();
            routingStatusValueLabel = new Label();
            routingStatusLabel = new Label();
            cableStateValueLabel = new Label();
            cableStateLabel = new Label();
            cableInputValueLabel = new Label();
            cableInputLabel = new Label();
            cableOutputValueLabel = new Label();
            cableOutputLabel = new Label();
            stopButton = new Button();
            startButton = new Button();
            refreshButton = new Button();
            bufferSizeComboBox = new DarkComboBox();
            bufferSizeLabel = new Label();
            outputDeviceComboBox = new DarkComboBox();
            outputDeviceLabel = new Label();
            inputDeviceComboBox = new DarkComboBox();
            inputDeviceLabel = new Label();
            mainPanel = new Panel();
            copyrightLabel = new Label();
            pluginChainGroupBox = new GroupBox();
            pluginChainListBox = new CheckedListBox();
            movePluginDownButton = new Button();
            movePluginUpButton = new Button();
            openPluginEditorButton = new Button();
            removePluginButton = new Button();
            addPluginButton = new Button();
            foundPluginsGroupBox = new GroupBox();
            foundPluginsListBox = new ListBox();
            pluginStatusLabel = new Label();
            scanPluginsButton = new Button();
            browsePluginFolderButton = new Button();
            pluginFolderTextBox = new TextBox();
            pluginFolderLabel = new Label();
            deleteProfileButton = new Button();
            saveProfileAsButton = new Button();
            saveProfileButton = new Button();
            profileComboBox = new DarkComboBox();
            profileLabel = new Label();
            settingsButton = new Button();
            leftPanel.SuspendLayout();
            outputLevelGroupBox.SuspendLayout();
            inputLevelGroupBox.SuspendLayout();
            virtualCableGroupBox.SuspendLayout();
            mainPanel.SuspendLayout();
            pluginChainGroupBox.SuspendLayout();
            foundPluginsGroupBox.SuspendLayout();
            SuspendLayout();
            // 
            // leftPanel
            // 
            leftPanel.BackColor = Color.FromArgb(231, 246, 244);
            leftPanel.Controls.Add(outputLevelGroupBox);
            leftPanel.Controls.Add(inputLevelGroupBox);
            leftPanel.Controls.Add(virtualCableGroupBox);
            leftPanel.Controls.Add(stopButton);
            leftPanel.Controls.Add(startButton);
            leftPanel.Controls.Add(refreshButton);
            leftPanel.Controls.Add(bufferSizeComboBox);
            leftPanel.Controls.Add(bufferSizeLabel);
            leftPanel.Controls.Add(outputDeviceComboBox);
            leftPanel.Controls.Add(outputDeviceLabel);
            leftPanel.Controls.Add(inputDeviceComboBox);
            leftPanel.Controls.Add(inputDeviceLabel);
            leftPanel.Dock = DockStyle.Left;
            leftPanel.Location = new Point(0, 0);
            leftPanel.Name = "leftPanel";
            leftPanel.Padding = new Padding(14);
            leftPanel.Size = new Size(320, 508);
            leftPanel.TabIndex = 0;
            // 
            // outputLevelGroupBox
            // 
            outputLevelGroupBox.Controls.Add(outputLevelStatusLabel);
            outputLevelGroupBox.Controls.Add(outputLevelMeter);
            outputLevelGroupBox.Location = new Point(14, 409);
            outputLevelGroupBox.Name = "outputLevelGroupBox";
            outputLevelGroupBox.Size = new Size(292, 89);
            outputLevelGroupBox.TabIndex = 9;
            outputLevelGroupBox.TabStop = false;
            outputLevelGroupBox.Text = "Output signal";
            // 
            // outputLevelStatusLabel
            // 
            outputLevelStatusLabel.AutoEllipsis = true;
            outputLevelStatusLabel.ForeColor = Color.FromArgb(104, 148, 153);
            outputLevelStatusLabel.Location = new Point(80, 23);
            outputLevelStatusLabel.Name = "outputLevelStatusLabel";
            outputLevelStatusLabel.Size = new Size(190, 20);
            outputLevelStatusLabel.TabIndex = 1;
            outputLevelStatusLabel.Text = "No output selected";
            // 
            // outputLevelMeter
            // 
            outputLevelMeter.BackColor = Color.FromArgb(4, 24, 33);
            outputLevelMeter.ForeColor = SystemColors.ControlDarkDark;
            outputLevelMeter.Location = new Point(14, 23);
            outputLevelMeter.Margin = new Padding(3, 2, 3, 2);
            outputLevelMeter.Name = "outputLevelMeter";
            outputLevelMeter.Size = new Size(65, 57);
            outputLevelMeter.TabIndex = 0;
            // 
            // inputLevelGroupBox
            // 
            inputLevelGroupBox.Controls.Add(inputLevelStatusLabel);
            inputLevelGroupBox.Controls.Add(inputLevelMeter);
            inputLevelGroupBox.Location = new Point(14, 314);
            inputLevelGroupBox.Name = "inputLevelGroupBox";
            inputLevelGroupBox.Size = new Size(292, 89);
            inputLevelGroupBox.TabIndex = 8;
            inputLevelGroupBox.TabStop = false;
            inputLevelGroupBox.Text = "Input signal";
            // 
            // inputLevelStatusLabel
            // 
            inputLevelStatusLabel.AutoEllipsis = true;
            inputLevelStatusLabel.ForeColor = Color.FromArgb(104, 148, 153);
            inputLevelStatusLabel.Location = new Point(80, 23);
            inputLevelStatusLabel.Name = "inputLevelStatusLabel";
            inputLevelStatusLabel.Size = new Size(190, 20);
            inputLevelStatusLabel.TabIndex = 1;
            inputLevelStatusLabel.Text = "No input selected";
            // 
            // inputLevelMeter
            // 
            inputLevelMeter.BackColor = Color.FromArgb(4, 24, 33);
            inputLevelMeter.ForeColor = SystemColors.ControlDarkDark;
            inputLevelMeter.Location = new Point(14, 23);
            inputLevelMeter.Margin = new Padding(3, 2, 3, 2);
            inputLevelMeter.Name = "inputLevelMeter";
            inputLevelMeter.Size = new Size(65, 57);
            inputLevelMeter.TabIndex = 0;
            // 
            // virtualCableGroupBox
            // 
            virtualCableGroupBox.Controls.Add(latencyStatusValueLabel);
            virtualCableGroupBox.Controls.Add(routingStatusValueLabel);
            virtualCableGroupBox.Controls.Add(routingStatusLabel);
            virtualCableGroupBox.Controls.Add(cableStateValueLabel);
            virtualCableGroupBox.Controls.Add(cableStateLabel);
            virtualCableGroupBox.Controls.Add(cableInputValueLabel);
            virtualCableGroupBox.Controls.Add(cableInputLabel);
            virtualCableGroupBox.Controls.Add(cableOutputValueLabel);
            virtualCableGroupBox.Controls.Add(cableOutputLabel);
            virtualCableGroupBox.Location = new Point(14, 185);
            virtualCableGroupBox.Name = "virtualCableGroupBox";
            virtualCableGroupBox.Size = new Size(292, 123);
            virtualCableGroupBox.TabIndex = 7;
            virtualCableGroupBox.TabStop = false;
            virtualCableGroupBox.Text = "Virtual cable";
            // 
            // latencyStatusValueLabel
            // 
            latencyStatusValueLabel.AutoEllipsis = true;
            latencyStatusValueLabel.ForeColor = Color.FromArgb(104, 148, 153);
            latencyStatusValueLabel.Location = new Point(14, 100);
            latencyStatusValueLabel.Name = "latencyStatusValueLabel";
            latencyStatusValueLabel.Size = new Size(263, 18);
            latencyStatusValueLabel.TabIndex = 9;
            latencyStatusValueLabel.Text = "q/o/c/p 0/0/0/100ms";
            // 
            // routingStatusValueLabel
            // 
            routingStatusValueLabel.AutoEllipsis = true;
            routingStatusValueLabel.Font = new Font("Segoe UI", 9F, FontStyle.Bold);
            routingStatusValueLabel.ForeColor = Color.FromArgb(104, 148, 153);
            routingStatusValueLabel.Location = new Point(87, 74);
            routingStatusValueLabel.Name = "routingStatusValueLabel";
            routingStatusValueLabel.Size = new Size(190, 17);
            routingStatusValueLabel.TabIndex = 7;
            routingStatusValueLabel.Text = "Stopped";
            // 
            // routingStatusLabel
            // 
            routingStatusLabel.AutoSize = true;
            routingStatusLabel.Location = new Point(14, 74);
            routingStatusLabel.Name = "routingStatusLabel";
            routingStatusLabel.Size = new Size(48, 20);
            routingStatusLabel.TabIndex = 6;
            routingStatusLabel.Text = "Route";
            // 
            // cableStateValueLabel
            // 
            cableStateValueLabel.AutoEllipsis = true;
            cableStateValueLabel.Font = new Font("Segoe UI", 9F, FontStyle.Bold);
            cableStateValueLabel.Location = new Point(87, 56);
            cableStateValueLabel.Name = "cableStateValueLabel";
            cableStateValueLabel.Size = new Size(190, 17);
            cableStateValueLabel.TabIndex = 5;
            cableStateValueLabel.Text = "Scanning";
            // 
            // cableStateLabel
            // 
            cableStateLabel.AutoSize = true;
            cableStateLabel.Location = new Point(14, 56);
            cableStateLabel.Name = "cableStateLabel";
            cableStateLabel.Size = new Size(49, 20);
            cableStateLabel.TabIndex = 4;
            cableStateLabel.Text = "Status";
            // 
            // cableInputValueLabel
            // 
            cableInputValueLabel.AutoEllipsis = true;
            cableInputValueLabel.Location = new Point(87, 38);
            cableInputValueLabel.Name = "cableInputValueLabel";
            cableInputValueLabel.Size = new Size(190, 17);
            cableInputValueLabel.TabIndex = 3;
            cableInputValueLabel.Text = "CABLE Output";
            // 
            // cableInputLabel
            // 
            cableInputLabel.AutoSize = true;
            cableInputLabel.Location = new Point(14, 38);
            cableInputLabel.Name = "cableInputLabel";
            cableInputLabel.Size = new Size(33, 20);
            cableInputLabel.TabIndex = 2;
            cableInputLabel.Text = "Mic";
            // 
            // cableOutputValueLabel
            // 
            cableOutputValueLabel.AutoEllipsis = true;
            cableOutputValueLabel.Location = new Point(87, 20);
            cableOutputValueLabel.Name = "cableOutputValueLabel";
            cableOutputValueLabel.Size = new Size(190, 17);
            cableOutputValueLabel.TabIndex = 1;
            cableOutputValueLabel.Text = "CABLE Input";
            // 
            // cableOutputLabel
            // 
            cableOutputLabel.AutoSize = true;
            cableOutputLabel.Location = new Point(14, 20);
            cableOutputLabel.Name = "cableOutputLabel";
            cableOutputLabel.Size = new Size(55, 20);
            cableOutputLabel.TabIndex = 0;
            cableOutputLabel.Text = "Output";
            // 
            // stopButton
            // 
            stopButton.Enabled = false;
            stopButton.Location = new Point(185, 143);
            stopButton.Name = "stopButton";
            stopButton.Size = new Size(80, 30);
            stopButton.TabIndex = 6;
            stopButton.Text = "Stop";
            stopButton.UseVisualStyleBackColor = true;
            stopButton.Click += StopButton_Click;
            // 
            // startButton
            // 
            startButton.Location = new Point(100, 143);
            startButton.Name = "startButton";
            startButton.Size = new Size(80, 30);
            startButton.TabIndex = 5;
            startButton.Text = "Start";
            startButton.UseVisualStyleBackColor = true;
            startButton.Click += StartButton_Click;
            // 
            // refreshButton
            // 
            refreshButton.Location = new Point(14, 143);
            refreshButton.Name = "refreshButton";
            refreshButton.Size = new Size(80, 30);
            refreshButton.TabIndex = 4;
            refreshButton.Text = "Refresh";
            refreshButton.UseVisualStyleBackColor = true;
            refreshButton.Click += RefreshButton_Click;
            // 
            // bufferSizeComboBox
            // 
            bufferSizeComboBox.BackColor = Color.FromArgb(4, 24, 33);
            bufferSizeComboBox.DrawMode = DrawMode.OwnerDrawFixed;
            bufferSizeComboBox.DropDownStyle = ComboBoxStyle.DropDownList;
            bufferSizeComboBox.FlatStyle = FlatStyle.Flat;
            bufferSizeComboBox.ForeColor = Color.FromArgb(231, 246, 244);
            bufferSizeComboBox.FormattingEnabled = true;
            bufferSizeComboBox.Location = new Point(87, 111);
            bufferSizeComboBox.Margin = new Padding(3, 2, 3, 2);
            bufferSizeComboBox.Name = "bufferSizeComboBox";
            bufferSizeComboBox.Size = new Size(80, 28);
            bufferSizeComboBox.TabIndex = 10;
            // 
            // bufferSizeLabel
            // 
            bufferSizeLabel.AutoSize = true;
            bufferSizeLabel.Font = new Font("Segoe UI", 9F, FontStyle.Bold);
            bufferSizeLabel.Location = new Point(14, 114);
            bufferSizeLabel.Name = "bufferSizeLabel";
            bufferSizeLabel.Size = new Size(54, 20);
            bufferSizeLabel.TabIndex = 11;
            bufferSizeLabel.Text = "Buffer";
            // 
            // outputDeviceComboBox
            // 
            outputDeviceComboBox.BackColor = Color.FromArgb(4, 24, 33);
            outputDeviceComboBox.DrawMode = DrawMode.OwnerDrawFixed;
            outputDeviceComboBox.DropDownStyle = ComboBoxStyle.DropDownList;
            outputDeviceComboBox.FlatStyle = FlatStyle.Flat;
            outputDeviceComboBox.ForeColor = Color.FromArgb(231, 246, 244);
            outputDeviceComboBox.FormattingEnabled = true;
            outputDeviceComboBox.Location = new Point(14, 78);
            outputDeviceComboBox.Name = "outputDeviceComboBox";
            outputDeviceComboBox.Size = new Size(293, 28);
            outputDeviceComboBox.TabIndex = 3;
            // 
            // outputDeviceLabel
            // 
            outputDeviceLabel.AutoSize = true;
            outputDeviceLabel.Font = new Font("Segoe UI", 9F, FontStyle.Bold);
            outputDeviceLabel.Location = new Point(14, 59);
            outputDeviceLabel.Name = "outputDeviceLabel";
            outputDeviceLabel.Size = new Size(105, 20);
            outputDeviceLabel.TabIndex = 2;
            outputDeviceLabel.Text = "OutputDevice";
            // 
            // inputDeviceComboBox
            // 
            inputDeviceComboBox.BackColor = Color.FromArgb(4, 24, 33);
            inputDeviceComboBox.DrawMode = DrawMode.OwnerDrawFixed;
            inputDeviceComboBox.DropDownStyle = ComboBoxStyle.DropDownList;
            inputDeviceComboBox.FlatStyle = FlatStyle.Flat;
            inputDeviceComboBox.ForeColor = Color.FromArgb(231, 246, 244);
            inputDeviceComboBox.FormattingEnabled = true;
            inputDeviceComboBox.Location = new Point(14, 30);
            inputDeviceComboBox.Name = "inputDeviceComboBox";
            inputDeviceComboBox.Size = new Size(293, 28);
            inputDeviceComboBox.TabIndex = 1;
            // 
            // inputDeviceLabel
            // 
            inputDeviceLabel.AutoSize = true;
            inputDeviceLabel.Font = new Font("Segoe UI", 9F, FontStyle.Bold);
            inputDeviceLabel.Location = new Point(14, 11);
            inputDeviceLabel.Name = "inputDeviceLabel";
            inputDeviceLabel.Size = new Size(93, 20);
            inputDeviceLabel.TabIndex = 0;
            inputDeviceLabel.Text = "InputDevice";
            // 
            // mainPanel
            // 
            mainPanel.BackColor = Color.White;
            mainPanel.Controls.Add(copyrightLabel);
            mainPanel.Controls.Add(pluginChainGroupBox);
            mainPanel.Controls.Add(addPluginButton);
            mainPanel.Controls.Add(foundPluginsGroupBox);
            mainPanel.Controls.Add(pluginStatusLabel);
            mainPanel.Controls.Add(scanPluginsButton);
            mainPanel.Controls.Add(browsePluginFolderButton);
            mainPanel.Controls.Add(pluginFolderTextBox);
            mainPanel.Controls.Add(pluginFolderLabel);
            mainPanel.Controls.Add(deleteProfileButton);
            mainPanel.Controls.Add(saveProfileAsButton);
            mainPanel.Controls.Add(saveProfileButton);
            mainPanel.Controls.Add(profileComboBox);
            mainPanel.Controls.Add(profileLabel);
            mainPanel.Controls.Add(settingsButton);
            mainPanel.Dock = DockStyle.Fill;
            mainPanel.ForeColor = SystemColors.ControlText;
            mainPanel.Location = new Point(320, 0);
            mainPanel.Name = "mainPanel";
            mainPanel.Padding = new Padding(20);
            mainPanel.Size = new Size(512, 500);
            mainPanel.TabIndex = 1;
            // 
            // copyrightLabel
            // 
            copyrightLabel.Anchor = AnchorStyles.Bottom | AnchorStyles.Right;
            copyrightLabel.Font = new Font("Segoe UI", 11F);
            copyrightLabel.ForeColor = Color.FromArgb(128, 176, 182);
            copyrightLabel.Location = new Point(150, 465);
            copyrightLabel.Name = "copyrightLabel";
            copyrightLabel.Size = new Size(342, 25);
            copyrightLabel.TabIndex = 8;
            copyrightLabel.Text = "VST-Applier by Yousef Mohiey (c) 2026";
            copyrightLabel.TextAlign = ContentAlignment.MiddleRight;
            copyrightLabel.Click += copyrightLabel_Click;
            // 
            // pluginChainGroupBox
            // 
            pluginChainGroupBox.Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right;
            pluginChainGroupBox.Controls.Add(pluginChainListBox);
            pluginChainGroupBox.Controls.Add(movePluginDownButton);
            pluginChainGroupBox.Controls.Add(movePluginUpButton);
            pluginChainGroupBox.Controls.Add(openPluginEditorButton);
            pluginChainGroupBox.Controls.Add(removePluginButton);
            pluginChainGroupBox.Location = new Point(20, 300);
            pluginChainGroupBox.Margin = new Padding(3, 2, 3, 2);
            pluginChainGroupBox.Name = "pluginChainGroupBox";
            pluginChainGroupBox.Padding = new Padding(3, 2, 3, 2);
            pluginChainGroupBox.Size = new Size(482, 168);
            pluginChainGroupBox.TabIndex = 7;
            pluginChainGroupBox.TabStop = false;
            pluginChainGroupBox.Text = "Plugin chain";
            // 
            // pluginChainListBox
            // 
            pluginChainListBox.Anchor = AnchorStyles.Top | AnchorStyles.Bottom | AnchorStyles.Left | AnchorStyles.Right;
            pluginChainListBox.FormattingEnabled = true;
            pluginChainListBox.Location = new Point(14, 23);
            pluginChainListBox.Margin = new Padding(3, 2, 3, 2);
            pluginChainListBox.Name = "pluginChainListBox";
            pluginChainListBox.Size = new Size(384, 110);
            pluginChainListBox.TabIndex = 0;
            pluginChainListBox.ItemCheck += pluginChainListBox_ItemCheck;
            pluginChainListBox.SelectedIndexChanged += pluginChainListBox_SelectedIndexChanged;
            pluginChainListBox.MouseDoubleClick += pluginChainListBox_MouseDoubleClick;
            pluginChainListBox.MouseDown += pluginChainListBox_MouseDown;
            // 
            // movePluginDownButton
            // 
            movePluginDownButton.Anchor = AnchorStyles.Top | AnchorStyles.Right;
            movePluginDownButton.Enabled = false;
            movePluginDownButton.Location = new Point(402, 51);
            movePluginDownButton.Margin = new Padding(3, 2, 3, 2);
            movePluginDownButton.Name = "movePluginDownButton";
            movePluginDownButton.Size = new Size(63, 23);
            movePluginDownButton.TabIndex = 2;
            movePluginDownButton.Text = "Down";
            movePluginDownButton.UseVisualStyleBackColor = true;
            movePluginDownButton.Click += movePluginDownButton_Click;
            // 
            // movePluginUpButton
            // 
            movePluginUpButton.Anchor = AnchorStyles.Top | AnchorStyles.Right;
            movePluginUpButton.Enabled = false;
            movePluginUpButton.Location = new Point(402, 23);
            movePluginUpButton.Margin = new Padding(3, 2, 3, 2);
            movePluginUpButton.Name = "movePluginUpButton";
            movePluginUpButton.Size = new Size(63, 23);
            movePluginUpButton.TabIndex = 1;
            movePluginUpButton.Text = "Up";
            movePluginUpButton.UseVisualStyleBackColor = true;
            movePluginUpButton.Click += movePluginUpButton_Click;
            // 
            // openPluginEditorButton
            // 
            openPluginEditorButton.Anchor = AnchorStyles.Top | AnchorStyles.Right;
            openPluginEditorButton.Enabled = false;
            openPluginEditorButton.Location = new Point(402, 106);
            openPluginEditorButton.Margin = new Padding(3, 2, 3, 2);
            openPluginEditorButton.Name = "openPluginEditorButton";
            openPluginEditorButton.Size = new Size(63, 23);
            openPluginEditorButton.TabIndex = 4;
            openPluginEditorButton.Text = "Editor";
            openPluginEditorButton.UseVisualStyleBackColor = true;
            openPluginEditorButton.Click += openPluginEditorButton_Click;
            // 
            // removePluginButton
            // 
            removePluginButton.Anchor = AnchorStyles.Top | AnchorStyles.Right;
            removePluginButton.Enabled = false;
            removePluginButton.Location = new Point(402, 79);
            removePluginButton.Margin = new Padding(3, 2, 3, 2);
            removePluginButton.Name = "removePluginButton";
            removePluginButton.Size = new Size(63, 23);
            removePluginButton.TabIndex = 3;
            removePluginButton.Text = "Remove";
            removePluginButton.UseVisualStyleBackColor = true;
            removePluginButton.Click += removePluginButton_Click;
            // 
            // addPluginButton
            // 
            addPluginButton.Anchor = AnchorStyles.Top | AnchorStyles.Right;
            addPluginButton.Enabled = false;
            addPluginButton.Location = new Point(378, 269);
            addPluginButton.Margin = new Padding(3, 2, 3, 2);
            addPluginButton.Name = "addPluginButton";
            addPluginButton.Size = new Size(116, 23);
            addPluginButton.TabIndex = 6;
            addPluginButton.Text = "Add plugin";
            addPluginButton.UseVisualStyleBackColor = true;
            addPluginButton.Click += addPluginButton_Click;
            // 
            // foundPluginsGroupBox
            // 
            foundPluginsGroupBox.Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right;
            foundPluginsGroupBox.Controls.Add(foundPluginsListBox);
            foundPluginsGroupBox.Location = new Point(20, 138);
            foundPluginsGroupBox.Margin = new Padding(3, 2, 3, 2);
            foundPluginsGroupBox.Name = "foundPluginsGroupBox";
            foundPluginsGroupBox.Padding = new Padding(3, 2, 3, 2);
            foundPluginsGroupBox.Size = new Size(482, 124);
            foundPluginsGroupBox.TabIndex = 5;
            foundPluginsGroupBox.TabStop = false;
            foundPluginsGroupBox.Text = "Found VST plugins";
            // 
            // foundPluginsListBox
            // 
            foundPluginsListBox.Anchor = AnchorStyles.Top | AnchorStyles.Bottom | AnchorStyles.Left | AnchorStyles.Right;
            foundPluginsListBox.FormattingEnabled = true;
            foundPluginsListBox.Location = new Point(14, 23);
            foundPluginsListBox.Margin = new Padding(3, 2, 3, 2);
            foundPluginsListBox.Name = "foundPluginsListBox";
            foundPluginsListBox.Size = new Size(454, 84);
            foundPluginsListBox.TabIndex = 0;
            foundPluginsListBox.SelectedIndexChanged += foundPluginsListBox_SelectedIndexChanged;
            foundPluginsListBox.MouseDoubleClick += foundPluginsListBox_MouseDoubleClick;
            // 
            // pluginStatusLabel
            // 
            pluginStatusLabel.Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right;
            pluginStatusLabel.AutoEllipsis = true;
            pluginStatusLabel.ForeColor = Color.FromArgb(104, 148, 153);
            pluginStatusLabel.Location = new Point(20, 271);
            pluginStatusLabel.Name = "pluginStatusLabel";
            pluginStatusLabel.Size = new Size(351, 19);
            pluginStatusLabel.TabIndex = 4;
            pluginStatusLabel.Text = "No VST plugins found";
            // 
            // scanPluginsButton
            // 
            scanPluginsButton.Anchor = AnchorStyles.Top | AnchorStyles.Right;
            scanPluginsButton.Location = new Point(412, 106);
            scanPluginsButton.Margin = new Padding(3, 2, 3, 2);
            scanPluginsButton.Name = "scanPluginsButton";
            scanPluginsButton.Size = new Size(82, 28);
            scanPluginsButton.TabIndex = 3;
            scanPluginsButton.Text = "Scan";
            scanPluginsButton.UseVisualStyleBackColor = true;
            scanPluginsButton.Click += scanPluginsButton_Click;
            // 
            // browsePluginFolderButton
            // 
            browsePluginFolderButton.Anchor = AnchorStyles.Top | AnchorStyles.Right;
            browsePluginFolderButton.Location = new Point(325, 106);
            browsePluginFolderButton.Margin = new Padding(3, 2, 3, 2);
            browsePluginFolderButton.Name = "browsePluginFolderButton";
            browsePluginFolderButton.Size = new Size(82, 28);
            browsePluginFolderButton.TabIndex = 2;
            browsePluginFolderButton.Text = "Browse";
            browsePluginFolderButton.UseVisualStyleBackColor = true;
            browsePluginFolderButton.Click += BrowsePluginFolderButton_Click;
            // 
            // pluginFolderTextBox
            // 
            pluginFolderTextBox.Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right;
            pluginFolderTextBox.Location = new Point(20, 108);
            pluginFolderTextBox.Margin = new Padding(3, 2, 3, 2);
            pluginFolderTextBox.Name = "pluginFolderTextBox";
            pluginFolderTextBox.Size = new Size(298, 27);
            pluginFolderTextBox.TabIndex = 1;
            // 
            // pluginFolderLabel
            // 
            pluginFolderLabel.AutoSize = true;
            pluginFolderLabel.Font = new Font("Segoe UI", 9F, FontStyle.Bold);
            pluginFolderLabel.ForeColor = SystemColors.ControlText;
            pluginFolderLabel.Location = new Point(20, 88);
            pluginFolderLabel.Name = "pluginFolderLabel";
            pluginFolderLabel.Size = new Size(99, 20);
            pluginFolderLabel.TabIndex = 0;
            pluginFolderLabel.Text = "Plugin folder";
            // 
            // deleteProfileButton
            // 
            deleteProfileButton.Anchor = AnchorStyles.Top | AnchorStyles.Right;
            deleteProfileButton.Enabled = false;
            deleteProfileButton.Location = new Point(424, 48);
            deleteProfileButton.Margin = new Padding(3, 2, 3, 2);
            deleteProfileButton.Name = "deleteProfileButton";
            deleteProfileButton.Size = new Size(66, 28);
            deleteProfileButton.TabIndex = 13;
            deleteProfileButton.Text = "Delete";
            deleteProfileButton.UseVisualStyleBackColor = true;
            deleteProfileButton.Click += deleteProfileButton_Click;
            // 
            // saveProfileAsButton
            // 
            saveProfileAsButton.Anchor = AnchorStyles.Top | AnchorStyles.Right;
            saveProfileAsButton.Location = new Point(342, 48);
            saveProfileAsButton.Margin = new Padding(3, 2, 3, 2);
            saveProfileAsButton.Name = "saveProfileAsButton";
            saveProfileAsButton.Size = new Size(78, 28);
            saveProfileAsButton.TabIndex = 12;
            saveProfileAsButton.Text = "Save as";
            saveProfileAsButton.UseVisualStyleBackColor = true;
            saveProfileAsButton.Click += saveProfileAsButton_Click;
            // 
            // saveProfileButton
            // 
            saveProfileButton.Anchor = AnchorStyles.Top | AnchorStyles.Right;
            saveProfileButton.Location = new Point(278, 48);
            saveProfileButton.Margin = new Padding(3, 2, 3, 2);
            saveProfileButton.Name = "saveProfileButton";
            saveProfileButton.Size = new Size(60, 28);
            saveProfileButton.TabIndex = 11;
            saveProfileButton.Text = "Save";
            saveProfileButton.UseVisualStyleBackColor = true;
            saveProfileButton.Click += saveProfileButton_Click;
            // 
            // profileComboBox
            // 
            profileComboBox.BackColor = Color.FromArgb(4, 24, 33);
            profileComboBox.DrawMode = DrawMode.OwnerDrawFixed;
            profileComboBox.DropDownStyle = ComboBoxStyle.DropDownList;
            profileComboBox.FlatStyle = FlatStyle.Flat;
            profileComboBox.ForeColor = Color.FromArgb(231, 246, 244);
            profileComboBox.FormattingEnabled = true;
            profileComboBox.Location = new Point(20, 50);
            profileComboBox.Margin = new Padding(3, 2, 3, 2);
            profileComboBox.Name = "profileComboBox";
            profileComboBox.Size = new Size(250, 28);
            profileComboBox.TabIndex = 10;
            profileComboBox.SelectedIndexChanged += profileComboBox_SelectedIndexChanged;
            // 
            // profileLabel
            // 
            profileLabel.AutoSize = true;
            profileLabel.Font = new Font("Segoe UI", 9F, FontStyle.Bold);
            profileLabel.ForeColor = SystemColors.ControlText;
            profileLabel.Location = new Point(20, 30);
            profileLabel.Name = "profileLabel";
            profileLabel.Size = new Size(57, 20);
            profileLabel.TabIndex = 9;
            profileLabel.Text = "Profile";
            // 
            // settingsButton
            // 
            settingsButton.Location = new Point(20, 471);
            settingsButton.Margin = new Padding(3, 2, 3, 2);
            settingsButton.Name = "settingsButton";
            settingsButton.Size = new Size(92, 26);
            settingsButton.TabIndex = 14;
            settingsButton.Text = "Settings";
            settingsButton.UseVisualStyleBackColor = true;
            settingsButton.Click += settingsButton_Click;
            // 
            // MainForm
            // 
            AutoScaleMode = AutoScaleMode.None;
            ClientSize = new Size(832, 500);
            Controls.Add(mainPanel);
            Controls.Add(leftPanel);
            FormBorderStyle = FormBorderStyle.FixedSingle;
            MaximizeBox = false;
            MaximumSize = new Size(850, 547);
            MinimumSize = new Size(850, 547);
            Name = "MainForm";
            Text = "VST-Applier v0.1.0";
            leftPanel.ResumeLayout(false);
            leftPanel.PerformLayout();
            outputLevelGroupBox.ResumeLayout(false);
            inputLevelGroupBox.ResumeLayout(false);
            virtualCableGroupBox.ResumeLayout(false);
            virtualCableGroupBox.PerformLayout();
            mainPanel.ResumeLayout(false);
            mainPanel.PerformLayout();
            pluginChainGroupBox.ResumeLayout(false);
            foundPluginsGroupBox.ResumeLayout(false);
            ResumeLayout(false);
        }

        #endregion

        private Panel leftPanel;
        private Label inputDeviceLabel;
        private DarkComboBox inputDeviceComboBox;
        private Label outputDeviceLabel;
        private DarkComboBox outputDeviceComboBox;
        private Button refreshButton;
        private Label bufferSizeLabel;
        private DarkComboBox bufferSizeComboBox;
        private Button startButton;
        private Button stopButton;
        private GroupBox virtualCableGroupBox;
        private Label cableOutputLabel;
        private Label cableOutputValueLabel;
        private Label cableInputLabel;
        private Label cableInputValueLabel;
        private Label cableStateLabel;
        private Label cableStateValueLabel;
        private Label routingStatusLabel;
        private Label routingStatusValueLabel;
        private Label latencyStatusValueLabel;
        private GroupBox inputLevelGroupBox;
        private Label inputLevelStatusLabel;
        private AudioLevelMeterControl inputLevelMeter;
        private GroupBox outputLevelGroupBox;
        private Label outputLevelStatusLabel;
        private AudioLevelMeterControl outputLevelMeter;
        private Panel mainPanel;
        private Label copyrightLabel;
        private Label pluginFolderLabel;
        private TextBox pluginFolderTextBox;
        private Button browsePluginFolderButton;
        private Button scanPluginsButton;
        private Label pluginStatusLabel;
        private GroupBox foundPluginsGroupBox;
        private ListBox foundPluginsListBox;
        private Button addPluginButton;
        private GroupBox pluginChainGroupBox;
        private CheckedListBox pluginChainListBox;
        private Button movePluginDownButton;
        private Button movePluginUpButton;
        private Button removePluginButton;
        private Button openPluginEditorButton;
        private Label profileLabel;
        private DarkComboBox profileComboBox;
        private Button saveProfileButton;
        private Button saveProfileAsButton;
        private Button deleteProfileButton;
        private Button settingsButton;
    }
}
