/**
 * Deluge.preferences.NetworkPage.js
 *
 * Copyright (c) Damien Churchill 2009-2010 <damoxc@gmail.com>
 *
 * This file is part of Deluge and is licensed under GNU General Public License 3.0, or later, with
 * the additional special exception to link portions of this program with the OpenSSL library.
 * See LICENSE for more details.
 */
Ext.namespace('Deluge.preferences');

/**
 * @class Deluge.preferences.Network
 * @extends Ext.form.FormPanel
 */
Deluge.preferences.Network = Ext.extend(Ext.form.FormPanel, {
    border: false,
    layout: 'form',
    title: _('Network'),
    header: false,

    initComponent: function () {
        Deluge.preferences.Network.superclass.initComponent.call(this);
        var optMan = deluge.preferences.getOptionsManager();

        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Incoming Interface'),
            style: 'margin-bottom: 5px; padding-bottom: 0px;',
            autoHeight: true,
            labelWidth: 1,
            defaultType: 'textfield',
        });
        optMan.bind(
            'listen_interface',
            fieldset.add({
                name: 'listen_interface',
                fieldLabel: '',
                labelSeparator: '',
                width: 200,
            })
        );

        var fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Incoming Port'),
            style: 'margin-bottom: 5px; padding-bottom: 0px;',
            autoHeight: true,
            labelWidth: 1,
            defaultType: 'checkbox',
        });
        optMan.bind(
            'random_port',
            fieldset.add({
                fieldLabel: '',
                labelSeparator: '',
                boxLabel: _('Use Random Port'),
                name: 'random_port',
                height: 22,
                listeners: {
                    check: {
                        fn: function (e, checked) {
                            this.listenPort.setDisabled(checked);
                        },
                        scope: this,
                    },
                },
            })
        );

        this.listenPort = fieldset.add({
            xtype: 'spinnerfield',
            name: 'listen_port',
            fieldLabel: '',
            labelSeparator: '',
            width: 75,
            strategy: {
                xtype: 'number',
                decimalPrecision: 0,
                minValue: 0,
                maxValue: 65535,
            },
        });
        optMan.bind('listen_ports', this.listenPort);

        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Outgoing Interface'),
            style: 'margin-bottom: 5px; padding-bottom: 0px;',
            autoHeight: true,
            labelWidth: 1,
            defaultType: 'textfield',
        });
        optMan.bind(
            'outgoing_interface',
            fieldset.add({
                name: 'outgoing_interface',
                fieldLabel: '',
                labelSeparator: '',
                width: 200,
            })
        );

        // The Outgoing Ports group was here. libtorrent 2.0 has no setting
        // for the source port of an outgoing connection, so `outgoing_ports`
        // and `random_outgoing_ports` were stored and never applied. The
        // outgoing *interface* above is real and stays.
        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Network Extras'),
            autoHeight: true,
            layout: 'table',
            layoutConfig: {
                // Two, not three: Peer Exchange used to fill the third cell of
                // the first row. With four boxes left, two columns keep the
                // indented ones in the second column where the class expects
                // them.
                columns: 2,
            },
            defaultType: 'checkbox',
        });
        optMan.bind(
            'upnp',
            fieldset.add({
                fieldLabel: '',
                labelSeparator: '',
                boxLabel: _('UPnP'),
                name: 'upnp',
            })
        );
        optMan.bind(
            'natpmp',
            fieldset.add({
                fieldLabel: '',
                labelSeparator: '',
                boxLabel: _('NAT-PMP'),
                ctCls: 'x-deluge-indent-checkbox',
                name: 'natpmp',
            })
        );
        optMan.bind(
            'lsd',
            fieldset.add({
                fieldLabel: '',
                labelSeparator: '',
                boxLabel: _('LSD'),
                name: 'lsd',
            })
        );
        optMan.bind(
            'dht',
            fieldset.add({
                fieldLabel: '',
                labelSeparator: '',
                boxLabel: _('DHT'),
                ctCls: 'x-deluge-indent-checkbox',
                name: 'dht',
            })
        );

        fieldset = this.add({
            xtype: 'fieldset',
            border: false,
            title: _('Type Of Service'),
            style: 'margin-bottom: 5px; padding-bottom: 0px;',
            bodyStyle: 'margin: 0px; padding: 0px',
            autoHeight: true,
            defaultType: 'textfield',
        });
        optMan.bind(
            'peer_tos',
            fieldset.add({
                name: 'peer_tos',
                fieldLabel: _('Peer TOS Byte:'),
                labelSeparator: '',
                width: 40,
            })
        );
    },
});
